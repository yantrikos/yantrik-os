//! A tab's file changed outside the editor, as the watch sees it: never last-writer-wins.

use serde_json::json;

use super::{act, file, modified, notice, outside_writes, press, settle, state, text, typed, with_file};
use crate::command::WindowAction;

#[test]
fn an_outside_edit_to_a_tab_with_nothing_unsaved_is_followed_and_undo_goes_back() {
    let (app, path) = with_file("follow", "first\n");
    outside_writes(&app, &path, "theirs\n");
    assert_eq!(text(&app), "theirs\n", "the tab shows the file as it is");
    assert!(!modified(&app));
    assert!(notice(&app).contains("changed outside the editor; the tab now shows the file"), "{}", notice(&app));
    press(&app, WindowAction::Undo);
    assert_eq!(text(&app), "first\n", "what it showed before is one undo away");
    assert!(modified(&app), "and unsaved against the file as it is now");
}

#[test]
fn our_own_save_is_not_an_outside_edit() {
    let (app, path) = with_file("ours", "first\n");
    typed(&app, "second\n");
    press(&app, WindowAction::Save);
    app.look_at_files();
    settle(&app);
    assert_eq!(notice(&app), "Saved");
    assert_eq!((text(&app), file(&path)), ("second\n".into(), "second\n".into()));
}

#[test]
fn an_outside_edit_under_unsaved_changes_is_a_conflict_said_at_once_and_never_written_over() {
    let (app, path) = with_file("conflict-seen", "first\n");
    typed(&app, "mine\n");
    outside_writes(&app, &path, "theirs\n");
    assert!(notice(&app).contains("changed outside the editor while this tab has unsaved changes"), "{}", notice(&app));
    assert_eq!(text(&app), "mine\n", "the person's text stays");
    let refused = act(&app, "save", json!({})).unwrap_err();
    assert!(refused.contains("changed on disk"), "{refused}");
    assert_eq!(file(&path), "theirs\n");
    // The other version put back by its program: the save may go ahead again.
    outside_writes(&app, &path, "first\n");
    act(&app, "save", json!({})).expect("the file holds what the tab last read again");
    assert_eq!(file(&path), "mine\n");
}

#[test]
fn a_file_deleted_outside_makes_its_text_unsaved_and_the_close_asks() {
    let (app, path) = with_file("gone", "only copy\n");
    assert!(!modified(&app));
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    settle(&app);
    assert!(modified(&app), "its text is on no disk now");
    assert!(notice(&app).contains("is not there any more"), "{}", notice(&app));
    app.run_writes();
    assert!(super::draft_file(&app).exists(), "the text is in its draft");
    app.close_requested(yantrik_app::app::WindowId::PRIMARY);
    assert_eq!(state(&app)["dialog"], 3, "closing asks rather than drop it");
    assert!(app.exit_reason().is_none());
}

#[test]
fn a_file_renamed_while_open_is_followed_to_its_new_name() {
    let (app, from) = with_file("renamed", "body\n");
    let to = from.with_file_name("pricing.txt");
    std::fs::rename(&from, &to).unwrap();
    app.look_at_files();
    settle(&app);
    assert_eq!(app.model(|m| m.path(m.active_tab()).map(|p| p.to_path_buf())), Some(to.clone()));
    assert_eq!(notice(&app), format!("This file moved. It is {} now.", to.display()));
    assert!(!modified(&app));
    typed(&app, "body, edited\n");
    press(&app, WindowAction::Save);
    assert_eq!(file(&to), "body, edited\n");
    assert!(!from.exists(), "nothing was written where it used to be");
    assert_eq!(state(&app)["title"], "pricing.txt");
}
