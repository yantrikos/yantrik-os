//! What Files does, and says, when it cannot open a folder.
//!
//! VM 520 sweep, 4 October: Files reopened on `~/longtask/recipes`, which had been deleted since,
//! and greeted the person with "Could not open /home/yantrik/longtask/recipes: No such file or
//! directory (os error 2)". A folder Files was already showing (a reopen, a refresh, Back) that
//! has gone away is not an error the person made: Files lands on the nearest folder above it that
//! still exists and says so in one line. A folder the person asked for that is not there is
//! still refused, in words, never with the raw `os error` number.
use std::io;
use std::path::{Path, PathBuf};

/// The nearest existing folder above `gone`: its parent, or that one's parent, and so on.
/// `home` when the walk finds nothing (which only an unreadable `/` could cause), else `/`.
pub(super) fn nearest_existing(gone: &Path, home: Option<&Path>) -> PathBuf {
    let mut at = gone.parent();
    while let Some(dir) = at {
        if dir.is_dir() {
            return dir.to_path_buf();
        }
        at = dir.parent();
    }
    home.filter(|h| h.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The one line Files shows after landing somewhere else: "~/a/b is gone; showing ~/a".
pub(super) fn gone_note(gone: &str, showing: &str) -> String {
    format!("{gone} is gone; showing {showing}.")
}

/// A folder that could not be opened, said in words. `shown` is the path as the person sees it
/// (`~/…`). Nothing here carries the `(os error N)` suffix `io::Error` prints.
pub(super) fn open_failure(shown: &str, e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => format!("There is no folder at {shown}."),
        io::ErrorKind::PermissionDenied => format!("You do not have permission to open {shown}."),
        _ => format!("{shown} could not be opened: {}.", without_os_code(e)),
    }
}

/// `io::Error`'s text without its trailing "(os error N)": "Input/output error".
fn without_os_code(e: &io::Error) -> String {
    let text = e.to_string();
    match text.rfind(" (os error ") {
        Some(at) if text.ends_with(')') => text[..at].to_string(),
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deleted_folder_lands_on_the_nearest_folder_still_there() {
        let root = std::env::temp_dir().join(format!("files-gone-{}", std::process::id()));
        let kept = root.join("longtask");
        std::fs::create_dir_all(&kept).unwrap();
        // ~/longtask/recipes/stage-2, with recipes deleted: two levels up is the answer.
        let gone = kept.join("recipes").join("stage-2");
        assert_eq!(nearest_existing(&gone, None), kept);
        assert_eq!(nearest_existing(&kept.join("recipes"), None), kept);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_person_never_reads_an_os_error_number() {
        let gone = io::Error::from_raw_os_error(2);
        assert!(gone.to_string().contains("os error 2"), "the raw text this replaces");
        let said = open_failure("~/longtask/recipes", &gone);
        assert_eq!(said, "There is no folder at ~/longtask/recipes.");
        let denied = open_failure("/root", &io::Error::from_raw_os_error(13));
        assert_eq!(denied, "You do not have permission to open /root.");
        let other = open_failure("/mnt/disk", &io::Error::from_raw_os_error(5));
        assert!(!other.contains("os error"), "{other}");
        assert!(other.starts_with("/mnt/disk could not be opened: "), "{other}");
        assert_eq!(
            gone_note("~/longtask/recipes", "~/longtask"),
            "~/longtask/recipes is gone; showing ~/longtask."
        );
    }
}
