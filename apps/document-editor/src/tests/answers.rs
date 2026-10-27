//! What each call answers, built from its own work, and what it refuses: the words, the keys and
//! the numbers yDoc has always given.

use serde_json::json;

use super::{act, none, notice, opened, press, scratch, started, state, text, typed, with_file};
use crate::command::WindowAction;

#[test]
fn set_content_append_find_and_replace_all_answer_as_they_always_did() {
    let app = started();
    let set = act(&app, "set_content", json!({ "text": "# Title\n\nalpha beta\nbeta gamma" })).unwrap();
    assert_eq!(set, json!({ "words": 6, "chars": 30, "dirty": true }));
    let added = act(&app, "append", json!({ "text": "delta\n" })).unwrap();
    assert_eq!(added, json!({ "appended_chars": 6, "words": 7, "chars": 37 }));
    assert_eq!(text(&app), "# Title\n\nalpha beta\nbeta gamma\ndelta\n", "a line is started before the addition");
    let found = act(&app, "find", json!({ "query": "BETA" })).unwrap();
    assert_eq!(found, json!({ "query": "BETA", "matches": 2 }));
    assert_eq!(state(&app)["find_matches"], 2);
    let replaced = act(&app, "replace_all", json!({ "find": "beta", "with": "B" })).unwrap();
    assert_eq!(replaced, json!({ "replaced": 2 }));
    assert_eq!(text(&app), "# Title\n\nalpha B\nB gamma\ndelta\n");
    let refused = act(&app, "replace_all", json!({ "find": "", "with": "x" })).unwrap_err();
    assert_eq!(refused, "Say what to find.");
    assert_eq!(notice(&app), "Say what to find.", "refused twice: to the caller and in the notice");
}

#[test]
fn new_answers_and_is_refused_over_unsaved_text() {
    let app = started();
    let fresh = act(&app, "new", none()).unwrap();
    assert_eq!(fresh, json!({ "new": true, "path": null }));
    act(&app, "append", json!({ "text": "keep me" })).unwrap();
    let refused = act(&app, "new", none()).unwrap_err();
    assert_eq!(refused, "This document has unsaved changes. Save it first, or discard them in the window.");
    assert_eq!(text(&app), "keep me");
}

#[test]
fn open_answers_what_it_opened_and_says_what_it_would_throw_away() {
    let app = started();
    let dir = scratch("open");
    let a = dir.join("a.md");
    std::fs::write(&a, "# A\n\nOne two three.\n\n## B\n").unwrap();
    let answer = act(&app, "open", json!({ "path": a.display().to_string() })).unwrap();
    assert_eq!(answer, json!({ "opened": a, "bytes": 26, "words": 7, "headings": 2 }));
    typed(&app, "# A\n\nOne two three. And a sentence I typed.\n\n## B\n");
    let b = dir.join("b.md");
    std::fs::write(&b, "# B\n").unwrap();
    let refused = act(&app, "open", json!({ "path": b.display().to_string() })).unwrap_err();
    assert!(refused.starts_with(&format!("Opening {} would throw away ", b.display())), "{refused}");
    assert!(refused.contains("characters of \u{201c}A\u{201d} that are not in "), "{refused}");
    assert!(refused.ends_with("Save this document first, or open again with discard=true."), "{refused}");
    assert_eq!(super::path_of(&app), Some(a.clone()), "nothing was replaced");
    let discarded = act(&app, "open", json!({ "path": b.display().to_string(), "discard": true })).unwrap();
    assert_eq!(discarded["opened"], json!(b));
    assert_eq!(text(&app), "# B\n");
    app.run_writes();
    assert!(!super::draft_file(&app).exists(), "the discarded text's draft is gone with it");
}

#[test]
fn an_open_that_cannot_read_leaves_the_document_alone() {
    let (app, path) = with_file("unreadable", "# Keep\n");
    typed(&app, "# Keep\n\nmore\n");
    let binary = path.with_file_name("binary.md");
    std::fs::write(&binary, [0u8, 159, 146, 150]).unwrap();
    let refused = act(&app, "open", json!({ "path": binary.display().to_string(), "discard": true })).unwrap_err();
    assert!(refused.contains("not valid UTF-8") || refused.contains("control"), "{refused}");
    assert_eq!(text(&app), "# Keep\n\nmore\n", "nothing is replaced until the file has been read");
}

#[test]
fn a_call_is_refused_while_the_window_asks_the_person_a_question() {
    let app = started();
    act(&app, "append", json!({ "text": "unsaved" })).unwrap();
    press(&app, WindowAction::New);
    assert_eq!(state(&app)["prompt_open"], true);
    for (name, args) in [
        ("append", json!({ "text": "x" })),
        ("set_content", json!({ "text": "x" })),
        ("find", json!({ "query": "x" })),
        ("new", none()),
    ] {
        let refused = act(&app, name, args).unwrap_err();
        assert_eq!(refused, "yDoc is asking the person about a file. Answer that prompt first.", "{name}");
    }
    assert_eq!(text(&app), "unsaved");
    // `show` reads and changes nothing, so it goes through.
    assert_eq!(act(&app, "show", none()).unwrap(), json!({ "shown": true }));
}

#[test]
fn text_that_is_not_a_document_is_refused_and_leaves_the_document_as_it_was() {
    let app = started();
    act(&app, "set_content", json!({ "text": "fine" })).unwrap();
    let refused = act(&app, "append", json!({ "text": "bad\u{1}byte" })).unwrap_err();
    assert!(refused.contains("control bytes"), "{refused}");
    assert_eq!(text(&app), "fine");
    let big = "x".repeat(crate::limits::MAX_BYTES + 1);
    assert!(act(&app, "set_content", json!({ "text": big })).unwrap_err().contains("limited to 1 MiB"));
    assert_eq!(text(&app), "fine");
    // The same bound holds for the keyboard: the page is put back.
    typed(&app, "worse\u{2}");
    assert_eq!(text(&app), "fine");
    assert!(notice(&app).contains("control bytes"), "{}", notice(&app));
}

#[test]
fn opening_through_the_dialog_reports_a_missing_file_in_the_dialog() {
    let app = started();
    let missing = scratch("missing").join("nope.md");
    press(&app, WindowAction::Open);
    press(&app, WindowAction::Confirm { path: missing.display().to_string() });
    assert!(app.model(|m| m.dialog_error.starts_with("Cannot open ")), "{}", app.model(|m| m.dialog_error.clone()));
    assert_eq!(state(&app)["prompt_open"], true, "the path can be corrected");
    opened(&app, &scratch("missing-2").join("yes.md"), "# Yes\n");
}
