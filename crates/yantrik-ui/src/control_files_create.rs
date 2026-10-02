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
                _ => "file",
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
        let io = body.find("control_files_create::make(").expect("make call");
        assert!(io < later, "the disk work belongs inside the answer_later closure");
        assert!(!body[..io].contains("create_dir") && !body[..io].contains("create_new"));
    }
}
