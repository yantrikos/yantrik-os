use std::os::unix::fs::symlink;

use serde_json::json;

use super::*;

/// A home of its own for each test, removed when the guard drops.
struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home(name: &str) -> (Home, PathBuf) {
    let dir = std::env::temp_dir().join(format!("yantrik-files-move-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["Desktop/Trips", "Documents", ".ssh", "Elsewhere"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::write(dir.join("Desktop/trip-plan.md"), "plan").unwrap();
    let home = dir.canonicalize().unwrap();
    (Home(home.clone()), home)
}

fn args(home: &Path, path: &str, to: &str) -> Value {
    json!({ "path": home.join(path).to_str().unwrap(), "to_folder": home.join(to).to_str().unwrap() })
}

fn mv(home: &Path, path: &str, to: &str) -> Result<Value, String> {
    perform(&plan(&args(home, path, to), home, true)?)
}

#[test]
fn a_move_puts_the_file_in_the_folder_and_says_where() {
    let (_d, home) = home("works");
    let said = mv(&home, "Desktop/trip-plan.md", "Desktop/Trips").unwrap();
    assert_eq!(said["now_at"], home.join("Desktop/Trips/trip-plan.md").to_str().unwrap());
    assert_eq!(said["kind"], "file");
    assert_eq!(said["settled"], true);
    assert!(!home.join("Desktop/trip-plan.md").exists());
    assert_eq!(std::fs::read_to_string(home.join("Desktop/Trips/trip-plan.md")).unwrap(), "plan");
    // A folder moves with what is in it.
    let said = mv(&home, "Desktop/Trips", "Documents").unwrap();
    assert_eq!(said["kind"], "folder");
    assert!(home.join("Documents/Trips/trip-plan.md").exists());
}

#[test]
fn a_move_never_overwrites() {
    let (_d, home) = home("no-replace");
    std::fs::write(home.join("Desktop/Trips/trip-plan.md"), "OLD").unwrap();
    let said = mv(&home, "Desktop/trip-plan.md", "Desktop/Trips").unwrap_err();
    assert!(said.contains("already has something called trip-plan.md"), "{said}");
    assert!(said.contains("nothing was replaced"), "{said}");
    assert_eq!(std::fs::read_to_string(home.join("Desktop/Trips/trip-plan.md")).unwrap(), "OLD");
    assert_eq!(std::fs::read_to_string(home.join("Desktop/trip-plan.md")).unwrap(), "plan");
    // The no-replace rename itself, not a check before a plain rename: a name that appears
    // after the plan was made is still not replaced.
    let p = plan(&args(&home, "Elsewhere", "Desktop"), &home, true).unwrap();
    std::fs::create_dir(&p.dst).unwrap();
    std::fs::write(p.dst.join("late"), "x").unwrap();
    assert!(perform(&p).unwrap_err().contains("already has"));
    assert!(p.dst.join("late").exists() && p.src.exists());
}

#[test]
fn a_move_stays_in_the_persons_home_and_out_of_protected_places() {
    let (_d, home) = home("roots");
    symlink("/etc", home.join("escape")).unwrap();
    let outside = std::env::temp_dir().join(format!("yantrik-move-outside-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("o.txt"), "o").unwrap();
    let outside_str = outside.to_str().unwrap();
    let h = home.to_str().unwrap();
    let cases = [
        // out of the roots, either end
        json!({ "path": "/etc/passwd", "to_folder": format!("{h}/Documents") }),
        json!({ "path": format!("{outside_str}/o.txt"), "to_folder": format!("{h}/Documents") }),
        json!({ "path": format!("{h}/Desktop/trip-plan.md"), "to_folder": outside_str }),
        // a link out of the home, as the folder and as the item
        json!({ "path": format!("{h}/Desktop/trip-plan.md"), "to_folder": format!("{h}/escape") }),
        json!({ "path": format!("{h}/escape/passwd"), "to_folder": format!("{h}/Documents") }),
        // protected places
        json!({ "path": format!("{h}/Desktop/trip-plan.md"), "to_folder": format!("{h}/.ssh") }),
        json!({ "path": format!("{h}/.ssh"), "to_folder": format!("{h}/Documents") }),
        // traversal
        json!({ "path": format!("{h}/Desktop/../../../etc/passwd"), "to_folder": format!("{h}/Documents") }),
        json!({ "path": format!("{h}/Desktop/trip-plan.md"), "to_folder": format!("{h}/Documents/../../etc") }),
        // not a folder / not there
        json!({ "path": format!("{h}/Desktop/trip-plan.md"), "to_folder": format!("{h}/Documents/nowhere") }),
        json!({ "path": format!("{h}/Desktop/trip-plan.md"), "to_folder": format!("{h}/Desktop/trip-plan.md") }),
        json!({ "path": format!("{h}/Desktop/missing.md"), "to_folder": format!("{h}/Documents") }),
        // not an absolute path
        json!({ "path": "Desktop/trip-plan.md", "to_folder": format!("{h}/Documents") }),
        json!({ "path": format!("{h}/Desktop/trip-plan.md") }),
    ];
    for case in cases {
        let err = plan(&case, &home, true).expect_err(&case.to_string());
        assert!(!err.is_empty(), "{case}");
    }
    assert!(outside.join("o.txt").exists());
    assert!(home.join("Desktop/trip-plan.md").exists());
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn a_folder_is_not_moved_into_itself_or_where_it_already_is() {
    let (_d, home) = home("loop");
    std::fs::create_dir_all(home.join("Desktop/Trips/Deep")).unwrap();
    for to in ["Desktop/Trips", "Desktop/Trips/Deep"] {
        let err = plan(&args(&home, "Desktop/Trips", to), &home, true).unwrap_err();
        assert!(err.contains("inside it") || err.contains("already in"), "{err}");
    }
    let err = plan(&args(&home, "Desktop/trip-plan.md", "Desktop"), &home, true).unwrap_err();
    assert!(err.contains("already in"), "{err}");
    // Through a link that leads back inside it.
    symlink(home.join("Desktop/Trips/Deep"), home.join("Documents/back")).unwrap();
    assert!(plan(&args(&home, "Desktop/Trips", "Documents/back"), &home, true).is_err());
    assert!(home.join("Desktop/Trips/Deep").exists());
}

#[test]
fn a_move_is_graded_sensitive_and_touches_no_screen() {
    let spec = action();
    assert_eq!(spec.permission, "sensitive");
    assert!(spec.deferred, "the disk work is off the UI thread");
    let body = include_str!("control_files_move.rs").split("#[cfg(test)]").next().unwrap();
    assert!(body.contains(".risk(\"sensitive\")") && !body.contains(".risk(\"safe\")") && !body.contains(".risk(\"standard\")"));
    for screen in ["App", "ui.", "ensure_files_screen", "set_current_screen", "invoke_", "file_browser", "clipboard", "get_file", "weak"] {
        let touched = body.lines().filter(|l| !l.trim_start().starts_with("//")).any(|l| l.contains(screen) && !l.contains("an `App`"));
        assert!(!touched, "control_files_move.rs reaches for the screen via `{screen}`");
    }
    assert!(body.contains("rename_no_replace"), "the move must go through the no-replace rename");
    assert!(!body.contains("fs::rename("), "a plain rename replaces");
    assert!(!body.contains("fs::copy(") && !body.contains("remove_file"), "a move does not copy and delete");
}

#[test]
fn a_minds_refusal_to_select_names_the_move() {
    let key = &cursor::Caller::Named("mind:test-move-1059".to_string());
    for action in ["files_select", "files_cut", "files_paste", "files_copy"] {
        let said = cursor::act(key, action, &json!({ "name": "a" })).unwrap_err();
        assert!(said.contains("files_move"), "{said}");
        assert!(said.contains("`path` and `to_folder`"), "{said}");
    }
}

/// Trashing is one act with one grade, whichever action does it: a caller that is not a Mind
/// (a harness running as the person) must meet the same card at `files_trash_selected` as at
/// `files_delete`. The person's own press is a callback, not these actions, and stays card-free.
#[test]
fn trashing_has_one_grade_for_both_actions() {
    let src = include_str!("control_files.rs");
    assert_eq!(cursor::TRASH_GRADE, "dangerous");
    let delete = &src[src.find("Action::new(\"files_delete\"").unwrap()..];
    assert!(delete[..delete.find("move |args|").unwrap()].contains(".risk(files_cursor::TRASH_GRADE)"));
    assert!(src.contains("if $name == \"files_trash_selected\" { spec.risk(files_cursor::TRASH_GRADE) }"));
}

#[test]
fn nothing_is_moved_out_of_a_hidden_or_startup_place() {
    let (_d, home) = home("hidden-source");
    for f in [".config/x/f", ".local/bin/f", ".local/share/Trash/files/f", ".mozilla/firefox/p/key4.db"] {
        let p = home.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "x").unwrap();
        let err = plan(&args(&home, f, "Documents"), &home, true).expect_err(f);
        assert!(!err.is_empty(), "{f}");
        assert!(p.exists() && !home.join("Documents").join(p.file_name().unwrap()).exists(), "{f} moved");
    }
    // The rule is the hidden-place one, in its own words.
    let err = plan(&args(&home, ".local/bin/f", "Documents"), &home, true).unwrap_err();
    assert!(err.contains("hidden_place"), "{err}");
}

#[test]
fn the_persons_own_folders_are_not_moved_by_a_mind() {
    let (_d, home) = home("home-folders");
    std::fs::create_dir_all(home.join("Archive")).unwrap();
    for f in ["Desktop", "Documents", "Archive"] {
        let err = plan(&args(&home, f, "Elsewhere"), &home, true).expect_err(f);
        assert!(err.contains("one of the person's own folders"), "{f}: {err}");
    }
    // What is inside one moves, and an ordinary folder in the home moves.
    plan(&args(&home, "Desktop/Trips", "Documents"), &home, true).unwrap();
    plan(&args(&home, "Elsewhere", "Documents"), &home, true).unwrap();
}

#[test]
fn nothing_is_moved_into_a_startup_place_or_a_tree_that_carries_one() {
    let (_d, home) = home("hidden-dest");
    for d in [".local/bin", "bin", ".hidden-but-plain", ".config", "Elsewhere/.git"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    std::fs::write(home.join("Elsewhere/.git/config"), "[core]").unwrap();
    symlink(home.join(".config"), home.join("lnk")).unwrap();
    for to in [".local/bin", "bin", ".hidden-but-plain", "lnk"] {
        let err = plan(&args(&home, "Desktop/trip-plan.md", to), &home, true).expect_err(to);
        assert!(!err.is_empty(), "{to}");
    }
    // A folder holding a `.git/config` lands nowhere new: its tree carries a hidden place.
    let err = plan(&args(&home, "Elsewhere", "Documents"), &home, true).unwrap_err();
    assert!(err.contains("hidden_place"), "{err}");
    // The checks are the ones that decide, not the filesystem: the same moves the person asks for
    // themselves are not held to them.
    plan(&args(&home, "Desktop/trip-plan.md", "bin"), &home, false).unwrap();
}
