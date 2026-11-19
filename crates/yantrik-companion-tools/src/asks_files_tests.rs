//! What is written is the file that was shown (#1313, M3, L3): the card says what it cannot show,
//! shows where a path really goes, and anything that changed while it waited is not written over.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::asks_files;
use crate::fake_shell::{Answer, FakeShell, First, Probe};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yantrik-files-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(&dir).unwrap()
}

fn allowed(path: &str) -> Result<String, String> {
    Ok(path.to_string())
}

fn s(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// A probe that does something to the world while the card is up.
fn during(act: impl Fn() + Send + Sync + 'static) -> Option<Probe> {
    Some(Arc::new(act))
}

#[test]
fn the_cards_say_what_they_cannot_show() {
    let dir = scratch("says");
    let file = dir.join("a.txt");
    std::fs::write(&file, "one\ntwo\n").unwrap();
    let shell = FakeShell::session(First::Pending, Answer::Denied);
    let _ = asks_files::overwrite_with(&allowed, &s(&file), &s(&file), "x\ny\nz");
    let change = shell.cards()[0]["args_json"]["change"].as_str().unwrap().to_string();
    assert!(change.starts_with("replaces the whole file (was 2 lines, 8 characters; becomes 3 lines, 5 characters)"), "{change}");
    assert!(change.ends_with("the first and last lines are shown"), "{change}");
    let _ = asks_files::edit_with(&allowed, &s(&file), &s(&file), 1, "one", "uno", "one\ntwo\n", "uno\ntwo\n");
    let change = shell.cards()[1]["args_json"]["change"].as_str().unwrap().to_string();
    assert!(change.contains("only the first and last lines of what goes and what comes are shown"), "{change}");
}

#[test]
fn a_path_that_leads_elsewhere_shows_where() {
    let dir = scratch("resolves");
    let real = dir.join("real.txt");
    std::fs::write(&real, "old").unwrap();
    let link = dir.join("link.txt");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let shell = FakeShell::session(First::Pending, Answer::Denied);
    let _ = asks_files::overwrite_with(&allowed, &s(&link), &s(&link), "new");
    let card = &shell.cards()[0]["args_json"];
    assert_eq!(card["resolves_to"], crate::asks::quoted(&s(&real)), "{card}");
    let plain = dir.join("plain.txt");
    std::fs::write(&plain, "old").unwrap();
    let _ = asks_files::overwrite_with(&allowed, &s(&plain), &s(&plain), "new");
    assert!(shell.cards()[1]["args_json"].get("resolves_to").is_none(), "a path that goes where it says shows no second line");
}

#[test]
fn a_file_changed_in_the_wait_is_not_overwritten() {
    let dir = scratch("changed");
    let file = dir.join("a.txt");
    std::fs::write(&file, "old").unwrap();
    let (touched, path) = (file.clone(), file.clone());
    let _shell = FakeShell::session_probing(First::Pending, Answer::Granted, during(move || std::fs::write(&touched, "someone else's work").unwrap()));
    let said = asks_files::overwrite_with(&allowed, &s(&path), &s(&path), "new").unwrap_err();
    assert!(said.contains("changed while the question was open") && said.contains("Nothing was changed"), "{said}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "someone else's work");
}

#[test]
fn a_link_swapped_in_during_the_wait_is_not_followed() {
    let dir = scratch("swapped");
    let file = dir.join("a.txt");
    let other = dir.join("other.txt");
    std::fs::write(&file, "old").unwrap();
    std::fs::write(&other, "not what was shown").unwrap();
    let (a, b) = (file.clone(), other.clone());
    let _shell = FakeShell::session_probing(
        First::Pending,
        Answer::Granted,
        during(move || {
            std::fs::remove_file(&a).unwrap();
            std::os::unix::fs::symlink(&b, &a).unwrap();
        }),
    );
    let said = asks_files::overwrite_with(&allowed, &s(&file), &s(&file), "new").unwrap_err();
    assert!(said.contains("changed while the question was open"), "{said}");
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "not what was shown");
}

#[test]
fn a_path_that_stops_passing_the_check_is_not_written() {
    let dir = scratch("check");
    let file = dir.join("a.txt");
    std::fs::write(&file, "old").unwrap();
    let _shell = FakeShell::session(First::Pending, Answer::Granted);
    let refuse = |_: &str| -> Result<String, String> { Err("Access denied: hidden".into()) };
    let said = asks_files::overwrite_with(&refuse, &s(&file), &s(&file), "new").unwrap_err();
    assert!(said.contains("no longer passes the check"), "{said}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "old");
}

#[test]
fn an_edit_is_not_made_on_a_file_that_is_not_what_it_was_made_from() {
    let dir = scratch("edit-changed");
    let file = dir.join("a.txt");
    std::fs::write(&file, "alpha beta").unwrap();
    // Changed before the card goes up: the edit was computed from something else.
    let shell = FakeShell::session(First::Pending, Answer::Granted);
    let said = asks_files::edit_with(&allowed, &s(&file), &s(&file), 1, "alpha", "x", "alpha beta (stale)", "x beta (stale)").unwrap_err();
    assert!(said.contains("changed") && shell.cards().is_empty(), "{said}");
    // Changed in the wait.
    let touched = file.clone();
    let _shell = FakeShell::session_probing(First::Pending, Answer::Granted, during(move || std::fs::write(&touched, "alpha beta, and more").unwrap()));
    let said = asks_files::edit_with(&allowed, &s(&file), &s(&file), 1, "alpha", "x", "alpha beta", "x beta").unwrap_err();
    assert!(said.contains("changed while the question was open"), "{said}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "alpha beta, and more", "the other change is not lost");
}

#[test]
fn a_new_file_is_never_truncated_unasked() {
    let dir = scratch("new");
    let file = dir.join("a.txt");
    let shell = FakeShell::session(First::Pending, Answer::Denied);
    asks_files::write_new_or_overwrite_with(&allowed, &s(&file), &s(&file), "first").unwrap();
    assert!(shell.cards().is_empty(), "a file that is not there is made without a question");
    // It is there now, as it is when one appears between the look and the write: asked, and kept.
    let said = asks_files::write_new_or_overwrite_with(&allowed, &s(&file), &s(&file), "second").unwrap_err();
    assert!(said.contains("said no"), "{said}");
    assert_eq!(shell.cards().len(), 1);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first");
}
