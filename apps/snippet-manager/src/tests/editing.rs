//! The window: the editor's typing kept before anything moves it, the buttons, the filters and the
//! collections, each pressed as the person presses them and read back from the file.

use serde_json::json;
use slint::Model as _;

use super::open;
use crate::store::{ALL, FAVORITES};
use crate::Command;

/// The fault the app was fixed for: typing, clicking another snippet and clicking back reverted
/// the typing. The typing is kept before the editor moves, and is on disk.
#[test]
fn typing_then_moving_to_another_snippet_keeps_the_typing() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Tail the shell's journal", "journalctl -fu yantrik-ui -n 200");
    assert!(f.app.model(|m| m.editor_dirty()));
    f.app.ui(move |ui| ui.invoke_snip_select(borrow));
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_detail_title().to_string()), "Borrow checker dance");
    let kept = f.on_disk().get(journal).cloned().unwrap();
    assert_eq!(kept.title, "Tail the shell's journal");
    assert_eq!(kept.code, "journalctl -fu yantrik-ui -n 200");
    f.app.ui(move |ui| ui.invoke_snip_select(journal));
    assert_eq!(f.app.ui(|ui| ui.get_detail_code().to_string()), "journalctl -fu yantrik-ui -n 200");
}

/// What the store tidies (spaces round a title, repeated tags) is what the editor shows once it
/// is saved, and a typing that changes nothing writes nothing.
#[test]
fn save_keeps_what_the_editor_holds_and_shows_it_tidied() {
    let f = open();
    let (journal, _) = f.seeded();
    f.app.run(Command::Select(journal));
    f.app.ui(move |ui| {
        ui.invoke_snip_save(journal, "  Journal  ".into(), "Shell".into(), "journalctl -b".into(), "logs,, LOGS, boot".into())
    });
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_detail_title().to_string()), "Journal");
    assert_eq!(f.app.ui(|ui| ui.get_detail_tags().to_string()), "logs, boot");
    assert_eq!(f.on_disk().get(journal).unwrap().code, "journalctl -b");
    let writes = f.disk.writes();
    f.app.run(Command::Select(journal));
    f.quiet();
    assert_eq!(f.disk.writes(), writes, "nothing changed, so nothing was written");
}

/// New makes a snippet that is visible: the search and the tag are cleared, Favourites gives way
/// to All, and it is written.
#[test]
fn new_is_visible_kept_and_open() {
    let f = open();
    f.seeded();
    f.app.run(Command::Search("borrow".into()));
    f.app.run(Command::TagFilter("rust".into()));
    f.app.run(Command::CollectionSelect(FAVORITES));
    f.app.ui(|ui| ui.invoke_snip_new());
    f.quiet();
    let (query, tag, collection, open) = f.app.model(|m| (m.query.clone(), m.tag.clone(), m.collection, m.open().map(|s| s.title.clone())));
    assert_eq!((query.as_str(), tag.as_str(), collection), ("", "", ALL));
    assert_eq!(open.as_deref(), Some("Untitled Snippet"));
    assert_eq!(f.on_disk().snippets().len(), 3);
    assert_eq!(f.app.ui(|ui| ui.get_detail_language().to_string()), "Rust");
}

/// The star and the language menu each save one field; the typing in the editor goes in first,
/// so neither costs the person their typing.
#[test]
fn the_star_and_the_language_menu_keep_the_typing_first() {
    let f = open();
    let (journal, _) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Typed then starred", "journalctl -k");
    f.app.ui(move |ui| ui.invoke_snip_toggle_favorite(journal));
    f.quiet();
    let kept = f.on_disk().get(journal).cloned().unwrap();
    assert!(kept.favorite);
    assert_eq!(kept.title, "Typed then starred");
    f.type_into("Typed then a language", "journalctl -k -b");
    f.app.ui(|ui| ui.invoke_snip_set_language("Bash".into()));
    f.quiet();
    let kept = f.on_disk().get(journal).cloned().unwrap();
    assert_eq!((kept.language.as_str(), kept.title.as_str()), ("Bash", "Typed then a language"));
    assert_eq!(f.app.ui(|ui| ui.get_detail_language().to_string()), "Bash");
}

#[test]
fn the_language_menu_with_nothing_open_says_so() {
    let f = open();
    f.app.ui(|ui| ui.invoke_snip_set_language("Go".into()));
    assert_eq!(f.notice(), "Open a snippet before choosing its language.");
    assert_eq!(f.app.ui(|ui| ui.get_notice().to_string()), "Open a snippet before choosing its language.");
}

/// The search box repaints the list on every key and never the editor, and the active tag chip
/// pressed again clears it.
#[test]
fn searching_and_the_chips_narrow_the_list_and_leave_the_editor_alone() {
    let f = open();
    let (journal, _) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Half typed", "journalctl");
    f.app.ui(|ui| ui.invoke_snip_search("BORROW".into()));
    assert_eq!(f.app.ui(|ui| ui.get_snippets().row_count()), 1);
    assert_eq!(f.app.ui(|ui| ui.get_match_count()), 1);
    assert_eq!(f.app.ui(|ui| ui.get_detail_title().to_string()), "Half typed", "the typing is still there");
    f.app.ui(|ui| ui.invoke_snip_search("".into()));
    assert_eq!(f.app.ui(|ui| ui.get_match_count()), -1, "nothing narrowing it");
    f.app.ui(|ui| ui.invoke_snip_tag_filter("logs".into()));
    assert_eq!(f.app.ui(|ui| ui.get_snippets().row_count()), 1);
    f.app.ui(|ui| ui.invoke_snip_tag_filter("LOGS".into()));
    assert_eq!(f.app.model(|m| m.tag.clone()), "", "a filter with no way off is a trap");
}

/// The "+" button always asks for "New Collection" and still works the second time; a snippet
/// made in a collection is in it; deleting the collection moves its snippets back to All and
/// says so.
#[test]
fn collections_are_made_filled_renamed_and_deleted_without_losing_a_snippet() {
    let f = open();
    f.app.ui(|ui| ui.invoke_collection_create("New Collection".into()));
    f.app.ui(|ui| ui.invoke_collection_create("New Collection".into()));
    f.quiet();
    let names: Vec<String> = f.on_disk().collections().iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, ["New Collection", "New Collection 2"]);
    let second = f.app.model(|m| m.collection);
    f.app.ui(|ui| ui.invoke_snip_new());
    f.quiet();
    let id = f.app.model(|m| m.selected);
    assert_eq!(f.on_disk().get(id).unwrap().collection, second);
    f.app.ui(move |ui| ui.invoke_collection_rename(second, "Infra".into()));
    f.quiet();
    assert_eq!(f.state()["collection"], json!("Infra"));
    f.app.ui(move |ui| ui.invoke_collection_delete(second));
    f.quiet();
    assert_eq!(f.notice(), "1 snippet(s) moved back to All Snippets.");
    assert_eq!(f.app.model(|m| m.collection), ALL);
    assert_eq!(f.on_disk().get(id).unwrap().collection, ALL, "the folder went, the code stayed");
}

/// A delete from the pane throws away the snippet open, and the editor is emptied.
#[test]
fn delete_from_the_pane_removes_the_one_open() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.app.ui(move |ui| ui.invoke_snip_delete(journal));
    f.quiet();
    assert!(f.on_disk().get(journal).is_none());
    assert!(f.on_disk().get(borrow).is_some());
    assert_eq!(f.app.ui(|ui| ui.get_detail_id()), -1);
    assert_eq!(f.state()["open"], json!(null));
}

/// An edit for a snippet the editor no longer holds (a late callback) is not taken for typing.
#[test]
fn typing_reported_for_another_snippet_is_not_taken() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.app.run(Command::Edited(crate::Editor { id: borrow, title: "Stale".into(), ..crate::Editor::none() }));
    assert!(!f.app.model(|m| m.editor_dirty()));
    assert_eq!(f.app.model(|m| m.editor.title.clone()), "Tail the journal");
}

/// The Mind's `save` keeps the person's typing first and lays its change on top, and is answered
/// with the snippet as kept.
#[test]
fn a_minds_save_keeps_the_persons_typing_first() {
    let f = open();
    let (journal, _) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("The person's title", "journalctl");
    let answer = f.act("save", json!({ "id": journal.to_string(), "code": "journalctl -f" })).unwrap();
    assert_eq!(answer["title"], json!("The person's title"));
    let kept = f.on_disk().get(journal).cloned().unwrap();
    assert_eq!((kept.title.as_str(), kept.code.as_str()), ("The person's title", "journalctl -f"));
    assert_eq!(f.app.ui(|ui| ui.get_detail_code().to_string()), "journalctl -f");
    let e = f.act("save", json!({ "id": journal.to_string() })).unwrap_err();
    assert!(e.starts_with("nothing to change"), "{e}");
    let e = f.act("save", json!({ "id": journal.to_string(), "collection": "Nope" })).unwrap_err();
    assert!(e.contains("this store has none yet"), "{e}");
}
