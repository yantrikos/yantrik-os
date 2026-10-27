//! Saving, saving as and exporting: written whole or not at all, never over a file the document
//! did not expect, never called saved when it is not, and answered from the call's own result.

use serde_json::json;
use yantrik_app::documents::disk::Step;

use super::{act, failing, file, modified, none, notice, path_of, press, scratch, settle, started, state, typed, with_file};
use crate::command::{Command, WindowAction};
use crate::update::closing::NOT_CLOSED;

#[test]
fn save_as_writes_the_file_and_answers_with_what_landed() {
    let app = started();
    act(&app, "set_content", json!({ "text": "# Plan\n\nBody.\n" })).unwrap();
    let path = scratch("save-as").join("plan.md");
    let answer = act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(answer["saved"], json!(path), "{answer}");
    assert_eq!(answer["bytes"], 14, "{answer}");
    assert!(answer["modified_unix"].as_i64().unwrap() > 0, "{answer}");
    assert_eq!(file(&path), "# Plan\n\nBody.\n");
    assert_eq!(path_of(&app), Some(path.clone()), "the document lives there now");
    assert!(!modified(&app));
    assert_eq!(state(&app)["path"], json!(path));
    assert_eq!(state(&app)["dirty"], false);
}

#[test]
fn save_writes_the_file_it_came_from_and_checks_it_first() {
    let (app, path) = with_file("save", "# One\n");
    typed(&app, "# Two\n");
    let answer = act(&app, "save", none()).unwrap();
    assert_eq!((answer["saved"].clone(), answer["bytes"].clone()), (json!(path), json!(6)));
    assert_eq!(file(&path), "# Two\n");
    assert_eq!(notice(&app), "", "a plain save leaves nothing to say");
}

#[test]
fn save_with_no_file_is_a_sentence_never_silence() {
    let app = started();
    act(&app, "set_content", json!({ "text": "# Notes on pricing\n" })).unwrap();
    let refused = act(&app, "save", none()).unwrap_err();
    assert!(refused.starts_with("This document has no file yet, so there is nowhere to save it. Use Save As"), "{refused}");
    assert!(refused.contains("notes-on-pricing.md"), "it names a place that would do: {refused}");
    assert_eq!(notice(&app), refused, "the window says it too");
    // The window's Save asks for a home instead.
    press(&app, WindowAction::Save);
    assert_eq!(state(&app)["prompt_open"], true);
}

#[test]
fn a_file_changed_on_disk_is_never_written_over() {
    let (app, path) = with_file("conflict", "first\n");
    typed(&app, "mine\n");
    std::fs::write(&path, "theirs\n").unwrap();
    let refused = act(&app, "save", none()).unwrap_err();
    assert!(refused.contains("changed on disk since it was opened"), "{refused}");
    assert!(refused.contains("Use Save As to keep both versions."), "{refused}");
    assert_eq!(file(&path), "theirs\n");
    assert!(modified(&app), "the draft is intact");
}

#[test]
fn save_as_onto_a_file_is_refused_unless_it_says_overwrite() {
    let (app, path) = with_file("exists", "original\n");
    let other = path.with_file_name("other.md");
    std::fs::write(&other, "someone else's\n").unwrap();
    let args = json!({ "path": other.display().to_string() });
    let refused = act(&app, "save_as", args.clone()).unwrap_err();
    assert!(refused.contains("already exists. Choose another name; nothing was overwritten."), "{refused}");
    assert_eq!(file(&other), "someone else's\n");
    let answer = act(&app, "save_as", json!({ "path": other.display().to_string(), "overwrite": true })).unwrap();
    assert_eq!(answer["saved"], json!(other));
    assert_eq!(file(&other), "original\n");
    assert_eq!(path_of(&app), Some(other));
}

#[test]
fn a_save_that_fails_is_not_called_saved_and_the_draft_stays() {
    let (app, path) = with_file("fails", "v1\n");
    let disk = failing(&app);
    disk.fail(Step::Write, "notes.md", Some(1));
    typed(&app, "v2\n");
    let refused = act(&app, "save", none()).unwrap_err();
    assert!(refused.starts_with("Save failed: injected failure"), "{refused}");
    assert!(refused.ends_with("Your draft is still open and unchanged."), "{refused}");
    assert_eq!(file(&path), "v1\n");
    assert!(modified(&app));
    app.run_writes();
    assert!(super::draft_file(&app).exists(), "the text is in its draft");
    // The disk back, the same save goes through.
    act(&app, "save", none()).unwrap();
    assert_eq!(file(&path), "v2\n");
}

#[test]
fn export_markdown_writes_a_copy_and_leaves_the_document_where_it_is() {
    let (app, path) = with_file("export", "# Doc\n\nText.\n");
    let copy = path.with_file_name("copy.md");
    let answer = act(&app, "export_markdown", json!({ "path": copy.display().to_string() })).unwrap();
    assert_eq!(answer["exported"], json!(copy), "{answer}");
    assert_eq!(answer["format"], "markdown");
    assert_eq!(answer["bytes"], 13);
    assert_eq!(file(&copy), "# Doc\n\nText.\n");
    assert_eq!(path_of(&app), Some(path), "the document stays in its own file");
    let refused = act(&app, "export_markdown", json!({ "path": copy.display().to_string() })).unwrap_err();
    assert!(refused.contains("already exists"), "a copy never replaces a file: {refused}");
}

#[test]
fn the_html_export_is_the_markdown_rendered_and_the_document_is_untouched() {
    let (app, path) = with_file("export-html", "# Doc\n\n**bold** & text\n");
    press(&app, WindowAction::ExportHtml);
    let suggested = app.model(|m| m.dialog_path.clone());
    assert!(suggested.ends_with("notes.html"), "{suggested}");
    let html = path.with_file_name("doc.html");
    press(&app, WindowAction::Confirm { path: html.display().to_string() });
    let written = file(&html);
    assert!(written.starts_with("<!doctype html>\n<meta charset=\"utf-8\">\n<title>Doc</title>"), "{written}");
    assert!(written.contains("<strong>bold</strong> &amp; text"), "{written}");
    assert_eq!(state(&app)["prompt_open"], false, "the dialog went away");
    assert_eq!(path_of(&app), Some(path));
}

#[test]
fn an_export_that_fails_stays_in_its_dialog_with_the_reason() {
    let (app, path) = with_file("export-fails", "text\n");
    press(&app, WindowAction::ExportMarkdown);
    press(&app, WindowAction::Confirm { path: path.display().to_string() });
    let error = app.model(|m| m.dialog_error.clone());
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(state(&app)["prompt_open"], true, "the path can be corrected rather than retyped");
    assert_eq!(file(&path), "text\n");
}

#[test]
fn a_save_in_flight_refuses_other_calls_and_each_call_is_answered_by_its_own_work() {
    let (app, path) = with_file("busy", "v1\n");
    typed(&app, "v2\n");
    let saving = app.act("save", none()).unwrap();
    assert_eq!(app.jobs_waiting(), 1, "the save is in flight");
    let refused = app.act("append", json!({ "text": "more\n" })).expect_err("refused while a file is written");
    assert!(refused.contains("`append` cannot run while yDoc is reading or writing a file"), "{refused}");
    assert_eq!(super::text(&app), "v2\n", "the refused call changed nothing");
    settle(&app);
    let answer = saving.settle().unwrap();
    assert_eq!((answer["saved"].clone(), answer["bytes"].clone()), (json!(path), json!(3)));
    // The next call has its own answer.
    let appended = act(&app, "append", json!({ "text": "more\n" })).unwrap();
    assert!(appended.get("saved").is_none(), "{appended}");
    assert_eq!(appended["appended_chars"], 5);
}

#[test]
fn typing_while_a_file_is_written_is_refused_and_the_page_put_back() {
    let (app, path) = with_file("typing-busy", "v1\n");
    typed(&app, "v2\n");
    // Not landed yet: the save job waits for the test.
    app.run(crate::Command::Window(WindowAction::Save));
    assert_eq!(app.jobs_waiting(), 1);
    typed(&app, "v3 typed during the save\n");
    assert_eq!(super::text(&app), "v2\n", "the text the save agreed with is the document's");
    assert!(notice(&app).contains("reading or writing a file"), "{}", notice(&app));
    settle(&app);
    assert_eq!(file(&path), "v2\n");
    assert!(!modified(&app), "nothing typed during the save is lost or half-saved");
}

#[test]
fn an_export_that_fails_under_a_pending_close_keeps_the_window_and_says_why() {
    let (app, path) = with_file("export-close", "# Doc\n");
    let disk = failing(&app);
    disk.fail(Step::Write, "copy.md", Some(1));
    let copy = path.with_file_name("copy.md");
    press(&app, WindowAction::ExportMarkdown);
    app.run(Command::Window(WindowAction::Confirm { path: copy.display().to_string() }));
    app.close_requested(yantrik_app::app::WindowId::PRIMARY);
    settle(&app);
    assert!(notice(&app).contains("injected failure"), "the failure is not wiped: {}", notice(&app));
    assert!(notice(&app).ends_with(NOT_CLOSED), "{}", notice(&app));
    assert!(!app.model(|m| m.close_when_done), "no close is left waiting");
    assert!(app.exit_reason().is_none() && !app.windows().is_empty(), "the window stayed");
}
