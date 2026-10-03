//! `files_new_folder` and `files_new_file` say what happened to the disk.
//!
//! On VM 520 a mind called `files_new_folder` and got "accepted: True, settled: False" with nothing
//! saying the folder was made. It then stat'd the path, found it there, and told the person "the
//! folder already exists, nothing to create" - denying its own success, turn after turn. The answer
//! now carries the file-system fact: `created` when this call made it, `existed` when it was
//! already there and nothing changed.
//!
//! The decision is the create call's own result (`EEXIST`), never a stat beside it, so a path that
//! appears between a check and a create cannot be reported the wrong way round.

use std::path::Path;

use serde_json::{json, Value};

/// [`make`], first re-judging where it will land. `dir` is a path string the UI thread checked
/// some time ago; something in its parent chain may have become a link since, and `create_dir`
/// and `O_EXCL` guard only the last component. So for a mind the folder is resolved here, in the
/// worker, and the home-paths verdict runs again on the resolved path: a swap that moved the
/// create outside what a mind may make is refused, not followed.
pub fn make_checked(dir: &Path, name: &str, folder: bool, mind: bool, home: &Path) -> Result<Value, String> {
    if !mind {
        return make(dir, name, folder);
    }
    let resolved = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // The home is resolved too, so a home that is itself reached through a link still contains
    // the folders under it.
    let home = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    crate::control_files_mind::make_verdict(&resolved.to_string_lossy(), name, &home)
        .map_err(|why| format!("{} changed under the check and is no longer somewhere a mind may make this: {why}", dir.display()))?;
    make(&resolved, name, folder)
}

/// What a folder holds, for `files_go`'s answer: how many entries, the first few names (folders end
/// in `/`), and how many more. Dot-files are left out as Files leaves them out. Does file I/O: call
/// it off the UI thread. A missing folder is an error that names the call that makes it.
pub fn listing(path: &str) -> Result<Value, String> {
    const FIRST: usize = 12;
    let full = crate::filebrowser::expand_home(path);
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let read = match std::fs::read_dir(&full) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(crate::control_files_mind::no_folder_at(path, &home))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => return Err(format!("{path} is a file, not a folder")),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Err(format!("{path} cannot be read: permission denied")),
        Err(e) => return Err(format!("{path}: {e}")),
    };
    let mut names: Vec<String> = read
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let folder = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(if folder { format!("{name}/") } else { name })
        })
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    let entries = names.len();
    let more = entries.saturating_sub(FIRST);
    names.truncate(FIRST);
    Ok(json!({ "path": full.display().to_string(), "exists": true, "entries": entries, "first": names, "more": more }))
}

/// Whether `name` asks for more than one folder level: `longtask/recipes`, or `~/longtask/recipes`.
pub fn is_nested(name: &str) -> bool {
    name.contains('/')
}

/// `files_new_folder` for a path of several levels, making the ones that are missing.
///
/// On VM 520 a mind asked to build `~/longtask/recipes` found `~/longtask` missing, was told only
/// "there is no folder", and never made either. A path is taken here relative to `dir`, or to the
/// home when it starts `~/`, and every level goes through [`make_checked`] in turn: each parent is
/// resolved after the level above it exists, and judged again, so a link anywhere on the way out
/// of the home is refused, not followed. `..` and `.` are refused outright.
pub fn make_nested_checked(dir: &Path, name: &str, mind: bool, home: &Path) -> Result<Value, String> {
    let (mut at, rest) = match name.strip_prefix("~/") {
        Some(rest) => (home.to_path_buf(), rest),
        None if name.starts_with('/') => {
            return Err(format!("`{name}` is an absolute path; give it relative to this folder, or as ~/…"))
        }
        None => (dir.to_path_buf(), name),
    };
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err(format!("`{name}` names no folder"));
    }
    if let Some(bad) = parts.iter().find(|p| **p == ".." || **p == ".") {
        return Err(format!("`{name}` has `{bad}` in it; name the folders plainly"));
    }
    let mut made_parents = Vec::new();
    let mut last = Value::Null;
    for (i, part) in parts.iter().enumerate() {
        let answer = make_checked(&at, part, true, mind, home)?;
        let shown = answer.get("created").or_else(|| answer.get("existed")).and_then(Value::as_str).unwrap_or_default().to_string();
        if answer.get("existed").is_some() && answer["kind"] != "directory" {
            return Err(format!("{shown} is a {}, not a folder, so nothing can be made inside it", answer["kind"].as_str().unwrap_or("file")));
        }
        if i + 1 < parts.len() && answer.get("created").is_some() {
            made_parents.push(shown.clone());
        }
        at = std::path::PathBuf::from(&shown);
        last = answer;
    }
    if let Some(map) = last.as_object_mut() {
        map.insert("made_parents".into(), json!(made_parents));
    }
    Ok(last)
}

/// Create `name` in `dir` and report what happened. Does file I/O: call it off the UI thread.
pub fn make(dir: &Path, name: &str, folder: bool) -> Result<Value, String> {
    crate::fileops::name(name)?;
    let path = dir.join(name);
    let made = if folder {
        std::fs::create_dir(&path)
    } else {
        std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map(|_| ())
    };
    let shown = path.display().to_string();
    match made {
        Ok(()) => Ok(json!({ "created": shown, "kind": if folder { "directory" } else { "file" } })),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // What is there, which may not be what was asked for: a file where a folder was wanted
            // is said so, not reported as if the folder existed.
            let kind = match std::fs::symlink_metadata(&path) {
                Ok(m) if m.is_dir() => "directory",
                Ok(m) if m.file_type().is_symlink() => "symlink",
                Ok(_) => "file",
                // Gone again before it could be looked at: say so rather than call it a file.
                Err(_) => "unknown",
            };
            Ok(json!({ "existed": shown, "kind": kind }))
        }
        Err(e) => Err(format!("{shown}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-files-create-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_new_folder_answers_created() {
        let dir = scratch("folder");
        let got = make(&dir, "Northwind", true).unwrap();
        assert_eq!(got["created"], dir.join("Northwind").display().to_string());
        assert_eq!(got["kind"], "directory");
        assert!(got.get("existed").is_none());
        assert!(dir.join("Northwind").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_file_answers_created_and_is_empty() {
        let dir = scratch("file");
        let got = make(&dir, "a.txt", false).unwrap();
        assert_eq!(got["created"], dir.join("a.txt").display().to_string());
        assert_eq!(got["kind"], "file");
        assert_eq!(std::fs::metadata(dir.join("a.txt")).unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_existing_folder_answers_existed_and_changes_nothing() {
        let dir = scratch("existing-folder");
        std::fs::create_dir(dir.join("keep")).unwrap();
        std::fs::write(dir.join("keep/inside"), "x").unwrap();
        let got = make(&dir, "keep", true).unwrap();
        assert_eq!(got["existed"], dir.join("keep").display().to_string());
        assert_eq!(got["kind"], "directory");
        assert!(got.get("created").is_none());
        assert_eq!(std::fs::read_to_string(dir.join("keep/inside")).unwrap(), "x");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_existing_file_is_not_truncated() {
        let dir = scratch("existing-file");
        std::fs::write(dir.join("notes.txt"), "precious").unwrap();
        let got = make(&dir, "notes.txt", false).unwrap();
        assert_eq!(got["existed"], dir.join("notes.txt").display().to_string());
        assert_eq!(got["kind"], "file");
        assert_eq!(std::fs::read_to_string(dir.join("notes.txt")).unwrap(), "precious");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_where_a_folder_was_asked_for_says_file() {
        let dir = scratch("mismatch");
        std::fs::write(dir.join("x"), "").unwrap();
        let got = make(&dir, "x", true).unwrap();
        assert_eq!(got["existed"], dir.join("x").display().to_string());
        assert_eq!(got["kind"], "file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_swapped_for_a_link_out_of_the_home_is_refused_for_a_mind() {
        let root = scratch("swap");
        let home = root.join("home");
        let outside = root.join("outside");
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        // The check saw ~/Documents; by the time the worker runs it is a link out of the home.
        let dir = home.join("Documents");
        std::fs::remove_dir(&dir).unwrap();
        std::os::unix::fs::symlink(&outside, &dir).unwrap();
        let err = make_checked(&dir, "x", true, true, &home).unwrap_err();
        assert!(err.contains("changed under the check"), "{err}");
        assert!(!outside.join("x").exists(), "nothing was made through the link");
        // A person is not held to the mind's rule.
        assert!(make_checked(&dir, "y", true, false, &home).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_that_is_still_in_the_home_is_made_for_a_mind() {
        let root = scratch("still");
        let home = root.join("home");
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        let got = make_checked(&home.join("Documents"), "ok", true, true, &home).unwrap();
        assert_eq!(got["kind"], "directory");
        assert!(home.join("Documents/ok").is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_names_and_missing_parents_are_honest_errors() {
        let dir = scratch("bad");
        for bad in ["", ".", "..", "a/b", "a\0b"] {
            assert!(make(&dir, bad, true).is_err(), "{bad:?}");
            assert!(make(&dir, bad, false).is_err(), "{bad:?}");
        }
        let err = make(&dir.join("nope"), "x", true).unwrap_err();
        assert!(err.contains("nope"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The answer is the disk's own fact by the time it is given, so these two actions are settled;
/// only the listing refresh trails. Declaring them `defers()` again would send a mind back to
/// guessing, which is the VM 520 bug.
#[cfg(test)]
mod settled_tests {
    #[test]
    fn the_create_actions_are_settled_and_off_the_ui_thread() {
        let src = include_str!("control_files.rs");
        for action in ["files_new_folder", "files_new_file"] {
            let at = src.find(&format!("\"{action}\"")).expect(action);
            let end = src[at..].find("make_here(").expect("hands off to make_here") + at;
            assert!(!src[at..end].contains(".defers()"), "{action} must answer settled");
        }
        let helper = src.find("fn make_here(").expect("make_here");
        let body = &src[helper..];
        let later = body.find("answer_later(").expect("answer_later");
        let io = body.find("control_files_create::make_checked(").expect("make call");
        assert!(io < later, "the disk work belongs inside the answer_later closure");
        assert!(!body[..io].contains("create_dir") && !body[..io].contains("create_new"));
    }
}

/// Dropping `.defers()` must not move a grade: neither action declares a `.risk(..)`, so both
/// publish the default, `standard`, as the docs row says and as they did when they deferred.
#[cfg(test)]
mod grade_tests {
    use yantrik_app_runtime::control::Action;

    #[test]
    fn the_create_actions_stay_standard() {
        assert_eq!(Action::new("a", "b").permission, "standard", "the default grade");
        assert_eq!(Action::new("a", "b").defers().permission, Action::new("a", "b").permission, "deferring never regrades");
        let src = include_str!("control_files.rs");
        for action in ["files_new_folder", "files_new_file"] {
            let at = src.find(&format!("\"{action}\"")).expect(action);
            let end = src[at..].find("make_here(").expect("hands off to make_here") + at;
            assert!(!src[at..end].contains(".risk("), "{action} declares no grade of its own, so it stays standard");
        }
    }
}

#[cfg(test)]
mod nested_tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-files-nested-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_nested_folder_makes_its_missing_parents_and_says_which() {
        let home = scratch("nested");
        let got = make_nested_checked(&home, "~/longtask/recipes", false, &home).unwrap();
        assert_eq!(got["created"], home.join("longtask").join("recipes").display().to_string());
        assert_eq!(got["made_parents"], serde_json::json!([home.join("longtask").display().to_string()]));
        assert!(home.join("longtask/recipes").is_dir());
        // Again: nothing is made, and it says the folder was already there.
        let again = make_nested_checked(&home, "longtask/recipes", false, &home).unwrap();
        assert_eq!(again["existed"], home.join("longtask").join("recipes").display().to_string());
        assert_eq!(again["made_parents"], serde_json::json!([]));
    }

    #[test]
    fn a_nested_folder_refuses_dot_dot_and_an_absolute_path() {
        let home = scratch("dots");
        assert!(make_nested_checked(&home, "a/../../etc", false, &home).unwrap_err().contains(".."));
        assert!(make_nested_checked(&home, "./a", false, &home).unwrap_err().contains("`.`"));
        assert!(make_nested_checked(&home, "/etc/x", false, &home).unwrap_err().contains("absolute"));
        assert!(!home.join("a").exists(), "nothing was made before the refusal");
    }

    #[test]
    fn a_nested_folder_under_a_file_is_refused_not_made() {
        let home = scratch("underfile");
        std::fs::write(home.join("notes"), b"x").unwrap();
        let err = make_nested_checked(&home, "notes/inner", false, &home).unwrap_err();
        assert!(err.contains("not a folder"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn for_a_mind_a_link_out_of_the_home_on_the_way_is_refused() {
        let root = scratch("linkout");
        let home = root.join("home");
        let outside = root.join("outside");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, home.join("away")).unwrap();
        let err = make_nested_checked(&home, "away/inner", true, &home).unwrap_err();
        assert!(!outside.join("inner").exists(), "nothing was made outside the home: {err}");
    }

}
