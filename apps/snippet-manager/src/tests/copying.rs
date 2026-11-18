//! Copy: the code onto the clipboard, the use count kept, and "Copied!" said for a moment. The
//! answer says that it copied, never what, nor how long it was.

use std::time::Duration;

use serde_json::json;

use super::open;
use crate::update::COPY_FEEDBACK;
use crate::Command;

#[test]
fn copy_puts_the_code_on_the_clipboard_and_counts_it() {
    let f = open();
    let (journal, _) = f.seeded();
    let answer = f.act("copy", json!({ "id": "journal" })).unwrap();
    assert_eq!(f.clipboard.texts(), ["journalctl -fu yantrik-ui"]);
    assert_eq!(
        answer,
        json!({
            "copied": "Tail the journal", "id": journal, "with": "wl-copy",
            "use_count": 1, "use_count_saved": true,
        })
    );
    assert_eq!(f.on_disk().get(journal).unwrap().use_count, 1);
    assert!(f.on_disk().get(journal).unwrap().used > 0);
    // "Copied!" on the button, for a moment, whoever copied.
    assert_eq!(f.state()["copied_recently"], json!(true));
    assert!(f.app.ui(|ui| ui.get_copy_feedback_visible()));
    f.app.advance(COPY_FEEDBACK + Duration::from_millis(1));
    assert!(!f.app.ui(|ui| ui.get_copy_feedback_visible()));
    assert_eq!(f.state()["copied_recently"], json!(false));
}

/// The button copies what the person is looking at: the typing goes in first. The Mind's copy is
/// of the stored snippet, and the person's typing is left as it is, unsaved.
#[test]
fn the_button_copies_the_typing_and_the_minds_copy_leaves_it_alone() {
    let f = open();
    let (journal, _) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Tail the journal", "journalctl -fu yantrik-ui --since today");
    f.act("copy", json!({ "id": journal.to_string() })).unwrap();
    assert_eq!(f.clipboard.texts(), ["journalctl -fu yantrik-ui"], "the stored code");
    assert!(f.app.model(|m| m.editor_dirty()), "the typing is still the person's, unsaved");
    assert_eq!(f.app.ui(|ui| ui.get_detail_code().to_string()), "journalctl -fu yantrik-ui --since today");
    f.app.ui(move |ui| ui.invoke_snip_copy(journal));
    f.quiet();
    assert_eq!(f.clipboard.texts().last().map(String::as_str), Some("journalctl -fu yantrik-ui --since today"));
    assert_eq!(f.on_disk().get(journal).unwrap().code, "journalctl -fu yantrik-ui --since today");
    assert_eq!(f.on_disk().get(journal).unwrap().use_count, 2);
}

/// A clipboard that refuses is said, and the button does not say "Copied!".
#[test]
fn a_copy_the_clipboard_refused_is_said_and_not_claimed() {
    let f = open();
    let (journal, _) = f.seeded();
    f.clipboard.refuse(Some("wl-copy could not be started (No such file or directory); nothing was copied"));
    let e = f.act("copy", json!({ "id": journal.to_string() })).unwrap_err();
    assert!(e.contains("nothing was copied"), "{e}");
    assert!(f.notice().contains("nothing was copied"));
    assert!(!f.app.ui(|ui| ui.get_copy_feedback_visible()));
    assert_eq!(f.on_disk().get(journal).unwrap().use_count, 0, "not counted");
    f.clipboard.refuse(None);
}

/// The code is on the clipboard even when its use count cannot be written: the call is answered,
/// with the count it had, and the button says what was not kept.
#[test]
fn a_use_count_that_cannot_be_written_does_not_unsay_the_copy() {
    let f = open();
    let (journal, _) = f.seeded();
    f.disk.refuse(true);
    let answer = f.act("copy", json!({ "id": journal.to_string() })).unwrap();
    assert_eq!(answer["use_count_saved"], json!(false));
    assert_eq!(answer["use_count"], json!(0));
    assert_eq!(f.clipboard.texts().len(), 1);
    assert_eq!(f.notice(), "", "the Mind's copy clears the notice, as it did");
    f.app.ui(move |ui| ui.invoke_snip_copy(journal));
    f.quiet();
    assert_eq!(f.notice(), "The code is on the clipboard, but the use count could not be saved.");
    assert_eq!(f.app.model(move |m| m.library.get(journal).unwrap().use_count), 0, "taken back, as the file has it");
    f.disk.refuse(false);
}

#[test]
fn nothing_to_copy_is_refused_in_words() {
    let f = open();
    let id = f.keep("Empty soon", "Rust", "x", "");
    f.act("save", json!({ "id": id.to_string(), "code": "" })).unwrap();
    let e = f.act("copy", json!({ "id": id.to_string() })).unwrap_err();
    assert_eq!(e, "`Empty soon` has no code to copy");
    let e = f.act("copy", json!({ "id": "no such thing" })).unwrap_err();
    assert!(e.starts_with("no snippet with id or title"), "{e}");
    assert!(f.clipboard.texts().is_empty());
}
