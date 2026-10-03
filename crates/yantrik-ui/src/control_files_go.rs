//! `files_go` answers what is at the path, settled.
//!
//! On VM 520 a mind asked to build ~/longtask/recipes called `files_go /home/yantrik` and got
//! "accepted: True, settled: False" with nothing about the folder. Unsure it had arrived, it
//! issued the same call five more times and never made anything; `files_go` to the missing
//! ~/longtask only said there was no folder, with no next step. The listing is now read on the
//! worker and the answer is the fact: where, how much, the first names, and for a missing folder
//! the call that makes it.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use yantrik_ipc_contracts::home_paths;

/// How many names the answer carries; a mind's describe budget is tight and `more` says the rest.
const FIRST: usize = 12;

/// Look at the folder `asked` (absolute, or `~/…`). Does file I/O: call it off the UI thread.
/// A mind is held to its home and outside its protected places (home_paths), links followed.
pub fn look(asked: &str, mind: bool, home: &Path) -> Result<Value, String> {
    if mind {
        // The same verdict `files_stat` gives; only "not a folder there" is worded here.
        let said = home_paths::stat(asked, home);
        match (&said["exists"], said["kind"].as_str()) {
            (Value::Bool(true), Some("directory")) => {}
            (Value::Bool(true), _) => return Err(format!("{asked} is not a folder")),
            (Value::Bool(false), _) => return Err(missing(&path_of(asked, home), home)),
            _ => {
                return Err(format!(
                    "a mind's Files stays in the person's home, outside its protected places; {asked} is {}",
                    said["reason"].as_str().unwrap_or("not one it may open")
                ))
            }
        }
    }
    let path = path_of(asked, home);
    if !path.is_absolute() {
        return Err(format!("{asked} is not an absolute path; give /… or ~/…"));
    }
    let shown = path.display().to_string();
    match std::fs::metadata(&path) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(format!("{shown} is not a folder")),
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(missing(&path, home)),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => return Err(format!("permission denied: {shown}")),
        Err(e) => return Err(format!("{shown}: {e}")),
    }
    let read = std::fs::read_dir(&path).map_err(|e| match e.kind() {
        ErrorKind::PermissionDenied => format!("permission denied: {shown}"),
        _ => format!("{shown}: {e}"),
    })?;
    // Hidden names stay out, as they do on the Files screen unless the person shows them.
    let mut names: Vec<(bool, String)> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let dir = e.path().is_dir();
            (!name.starts_with('.')).then_some((dir, name))
        })
        .collect();
    names.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let total = names.len();
    let first: Vec<String> =
        names.into_iter().take(FIRST).map(|(dir, n)| if dir { format!("{n}/") } else { n }).collect();
    Ok(json!({ "path": shown, "exists": true, "entries": total, "first": first, "more": total - first.len() }))
}

fn path_of(asked: &str, home: &Path) -> PathBuf {
    home_paths::expand(asked.trim(), home).unwrap_or_else(|| PathBuf::from(asked))
}

/// The honest refusal for a missing folder, with the call that makes it when it is in the home.
fn missing(path: &Path, home: &Path) -> String {
    let shown = path.display().to_string();
    match path.strip_prefix(home) {
        Ok(rel) if !rel.as_os_str().is_empty() => format!(
            "there is no folder at {shown}; to make it, call files_new_folder with name \"~/{}\" \
             (missing parent folders are made too)",
            rel.display()
        ),
        _ => format!("there is no folder at {shown}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-files-go-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn go_is_settled_with_what_is_there() {
        let home = scratch("entries");
        std::fs::create_dir_all(home.join("docs/sub")).unwrap();
        for n in 0..14 {
            std::fs::write(home.join(format!("docs/f{n:02}.txt")), "").unwrap();
        }
        std::fs::write(home.join("docs/.hidden"), "").unwrap();
        for mind in [false, true] {
            let got = look(&home.join("docs").display().to_string(), mind, &home).unwrap();
            assert_eq!(got["exists"], true);
            assert_eq!(got["path"], home.join("docs").display().to_string());
            assert_eq!(got["entries"], 15, "14 files and a folder, no hidden");
            assert_eq!(got["first"].as_array().unwrap().len(), 12);
            assert_eq!(got["first"][0], "sub/", "folders first, marked with a slash");
            assert_eq!(got["more"], 3);
        }
        let got = look("~/docs", true, &home).unwrap();
        assert_eq!(got["entries"], 15, "~/ is the home");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_missing_folder_names_the_next_step() {
        let home = scratch("missing");
        for mind in [false, true] {
            let err = look(&home.join("longtask").display().to_string(), mind, &home).unwrap_err();
            assert!(err.contains(&format!("there is no folder at {}", home.join("longtask").display())), "{err}");
            assert!(err.contains("files_new_folder with name \"~/longtask\""), "{err}");
            assert!(err.contains("missing parent folders are made too"), "{err}");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_file_is_not_a_folder() {
        let home = scratch("notdir");
        std::fs::write(home.join("a.txt"), "").unwrap();
        for mind in [false, true] {
            let err = look(&home.join("a.txt").display().to_string(), mind, &home).unwrap_err();
            assert!(err.contains("is not a folder"), "{err}");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_folder_nobody_may_read_says_permission_denied() {
        use std::os::unix::fs::PermissionsExt;
        let home = scratch("denied");
        std::fs::create_dir(home.join("locked")).unwrap();
        std::fs::set_permissions(home.join("locked"), std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads anything, so only judge it when the lock holds.
        if std::fs::read_dir(home.join("locked")).is_err() {
            let err = look(&home.join("locked").display().to_string(), false, &home).unwrap_err();
            assert!(err.starts_with("permission denied"), "{err}");
        }
        std::fs::set_permissions(home.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_mind_is_still_kept_to_the_home() {
        let home = scratch("outside");
        let err = look("/etc", true, &home).unwrap_err();
        assert!(err.contains("stays in the person's home"), "{err}");
        let _ = std::fs::remove_dir_all(&home);
    }
}

/// `files_go` reads the disk on the worker and answers settled: it must not defer, and nothing
/// before `answer_later` may touch the file system.
#[cfg(test)]
mod settled_tests {
    #[test]
    fn files_go_is_settled_and_off_the_ui_thread() {
        let src = include_str!("control_files.rs");
        let at = src.find("\"files_go\"").expect("files_go");
        let end = src[at..].find("go_here(").expect("hands off to go_here") + at;
        assert!(!src[at..end].contains(".defers()"), "files_go must answer settled");
        assert!(!src[at..end].contains(".risk("), "files_go stays standard");
        let helper = src.find("fn go_here(").expect("go_here");
        let body = &src[helper..];
        let later = body.find("answer_later(").expect("answer_later");
        let io = body.find("control_files_go::look(").expect("look call");
        assert!(io < later, "the listing belongs inside the answer_later closure");
        assert!(!body[..io].contains("read_dir") && !body[..io].contains("metadata"));
    }

    #[test]
    fn files_go_stays_standard() {
        use yantrik_app_runtime::control::Action;
        assert_eq!(Action::new("a", "b").permission, "standard");
    }
}
