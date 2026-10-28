//! Tabs: the 32-tab cap (#683, 47c1aba8), where a new tab opens, switching, closing and restart.

use serde_json::json;
use slint::Model as _;

use super::{folder, open};
use crate::command::{Command, Do};
use crate::model::{tabs_full, MAX_TABS};

const FULL: &str = "Terminal has 32 tabs. Close a tab before opening another.";

#[test]
fn the_tab_limit_is_the_constant_and_the_refusal_names_it() {
    assert_eq!(MAX_TABS, 32);
    assert_eq!(tabs_full(MAX_TABS - 1), None);
    let full = tabs_full(MAX_TABS).expect("the cap should refuse at MAX_TABS");
    assert_eq!(full, FULL);
    assert!(full.contains(&MAX_TABS.to_string()), "the refusal does not name the real number: {full}");
    assert!(full.contains("Close a tab"), "{full}");
}

/// The cap, reached through the action and through the button, with real shells: the 33rd is
/// refused in silence nowhere. The caller is told, the notice says it, and no shell starts.
#[test]
fn the_thirty_third_tab_is_refused_to_the_caller_and_on_the_notice_line() {
    let f = open();
    f.open_tabs(MAX_TABS - 1);
    assert_eq!(f.tabs(), MAX_TABS);
    let pids = f.pids();

    let refused = f.act("new_tab", json!({})).unwrap_err();
    assert_eq!(refused, FULL);
    assert_eq!(f.notice(), FULL, "and the person at the window sees it too");
    assert_eq!(f.state()["notice"], json!(FULL));

    f.app.ui(|ui| ui.invoke_action("new".into()));
    f.quiet();
    assert_eq!(f.tabs(), MAX_TABS, "the New tab button cannot pass it either");
    let refused = f.act("open_directory", json!({ "directory": "/" })).unwrap_err();
    assert_eq!(refused, FULL, "and neither can open_directory");
    assert_eq!(f.pids(), pids, "no shell was started");
    assert_eq!(f.state()["tabs"], json!(MAX_TABS));
}

/// Tabs on their way count against the cap, so a burst cannot go past it.
#[test]
fn a_burst_of_requests_cannot_pass_the_cap() {
    let f = open();
    let pending: Vec<_> = (0..40).map(|_| f.app.act("new_tab", json!({})).unwrap()).collect();
    f.quiet();
    let answers: Vec<_> = pending.into_iter().map(|p| p.settle()).collect();
    assert_eq!(answers.iter().filter(|a| a.is_ok()).count(), MAX_TABS - 1);
    assert!(answers.iter().filter_map(|a| a.as_ref().err()).all(|e| e == FULL));
    assert_eq!(f.tabs(), MAX_TABS);
}

#[test]
fn closing_a_tab_makes_room_for_another() {
    let f = open();
    f.open_tabs(MAX_TABS - 1);
    f.app.run(Command::CloseTab(5));
    f.until("the tab gone", |f| f.tabs() == MAX_TABS - 1);
    f.act("new_tab", json!({})).unwrap();
    assert_eq!(f.tabs(), MAX_TABS);
}

#[test]
fn a_new_tab_opens_where_the_active_shell_is() {
    let f = open();
    let dir = folder(&f.paths, "folder with spaces; literal");
    f.act("run", json!({ "command": format!("cd '{}'", dir.display()) })).unwrap();
    f.until("the probe to see the folder", |f| f.state()["directory"] == json!(dir.display().to_string()));
    let opened = f.act("new_tab", json!({})).unwrap();
    assert_eq!(opened["directory"], json!(dir.display().to_string()));
    assert_eq!(opened["tabs"], json!(2));
    assert_eq!(opened["tab"], json!(1));
    assert_ne!(opened["shell_pid"], json!(f.pids()[0]), "a new tab is a new shell");
    assert_eq!(f.state()["active_tab"], json!(1), "and it is the one in front");
}

#[test]
fn open_directory_opens_a_shell_there_and_brings_the_window_forward() {
    let f = open();
    let dir = folder(&f.paths, "work dir");
    let given = format!("  {}  ", dir.display());
    let opened = f.act("open_directory", json!({ "directory": given })).unwrap();
    assert_eq!(opened["directory"], json!(dir.display().to_string()));
    assert_eq!(opened["asked_for"], json!(dir.display().to_string()));
    assert_eq!(opened["tabs"], json!(2));
    assert_eq!(opened["alive"], json!(true));
    assert_eq!(f.app.model(|m| m.raises), 1, "the window is asked to come forward");
}

#[test]
fn open_directory_refuses_what_it_always_refused_in_the_sentences_it_always_used() {
    let f = open();
    let relative = f.act("open_directory", json!({ "directory": "somewhere/relative" })).unwrap_err();
    assert_eq!(relative, "`directory` has to be an absolute path; `somewhere/relative` is not one.");
    assert_eq!(f.notice(), relative);
    let blank = f.act("open_directory", json!({ "directory": "   " })).unwrap_err();
    assert_eq!(blank, "`open_directory` needs `directory`: an absolute path to a folder.");
    let missing = f.act("open_directory", json!({ "directory": "/no/such/folder-here" })).unwrap_err();
    assert!(missing.starts_with("Could not open folder: "), "{missing}");
    assert_eq!(f.notice(), missing);
    let file = folder(&f.paths, "x").join("a-file");
    std::fs::write(&file, "x").unwrap();
    let not_a_folder = f.act("open_directory", json!({ "directory": file.display().to_string() })).unwrap_err();
    assert_eq!(not_a_folder, "The requested path is not a folder.");
    assert_eq!(f.tabs(), 1, "nothing was started by any of them");
}

#[test]
fn switching_and_closing_tabs_keeps_the_right_one_in_front() {
    let f = open();
    f.open_tabs(2);
    assert_eq!(f.state()["active_tab"], json!(2));
    let third = f.pids()[2];
    f.app.ui(|ui| ui.invoke_select_tab(0));
    f.quiet();
    assert_eq!(f.state()["active_tab"], json!(0));
    f.app.run(Command::Do(Do::NextTab));
    f.app.run(Command::Do(Do::PreviousTab));
    f.app.run(Command::Do(Do::PreviousTab));
    assert_eq!(f.state()["active_tab"], json!(2), "wraps from the first back to the last");
    // Closing a tab before the active one keeps the active shell active.
    f.app.ui(|ui| ui.invoke_close_tab(0));
    f.until("the tab gone", |f| f.tabs() == 2);
    assert_eq!(f.active_pid(), third);
    assert_eq!(f.state()["active_tab"], json!(1));
    assert!(!f.app.ui(|ui| ui.get_confirm_close()), "an idle shell closes without a question");
}

#[test]
fn restart_replaces_an_exited_shell_and_does_nothing_for_a_live_one() {
    let f = open();
    let first = f.active_pid();
    f.app.ui(|ui| ui.invoke_action("restart".into()));
    f.quiet();
    assert_eq!(f.active_pid(), first, "a live shell is not restarted");
    f.type_until("exit 3\r", "exit 3");
    f.until("the exit", |f| !f.app.model(|m| m.session().is_some_and(|s| s.alive())));
    f.app.ui(|ui| ui.invoke_action("restart".into()));
    f.until("a new shell", |f| f.tabs() == 1 && f.app.model(|m| m.session().is_some_and(|s| s.alive())));
    assert_ne!(f.active_pid(), first);
}

#[test]
fn the_tab_strip_and_the_window_follow_the_shells() {
    let f = open();
    let dir = folder(&f.paths, "titled");
    f.act("open_directory", json!({ "directory": dir.display().to_string() })).unwrap();
    let tabs = f.app.ui(|ui| ui.get_tabs().iter().map(|t| (t.title.to_string(), t.active, t.alive)).collect::<Vec<_>>());
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[1], ("titled".to_string(), true, true));
    assert!(!tabs[0].1);
    assert_eq!(f.app.ui(|ui| ui.get_directory().to_string()), dir.display().to_string());
}
