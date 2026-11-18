//! Paths as a person or a mind wrote them, and where a save is first offered.

use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// A path as a person or a mind wrote it: `~/` is the home, and a relative path is from `base`.
pub fn resolve(text: &str, base: &Path) -> PathBuf {
    let text = text.trim();
    let path = match text.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if text == "~" => home(),
        None => PathBuf::from(text),
    };
    if path.is_absolute() { path } else { base.join(path) }
}

/// A sensible place to suggest: the open file's, else `~/Documents` if there is one, else the
/// home. Reads the disk, so it runs in a job.
pub fn suggested(current: Option<&Path>, name: &str) -> PathBuf {
    if let Some(p) = current {
        return p.to_path_buf();
    }
    let home = home();
    let docs = home.join("Documents");
    let dir = if docs.is_dir() { docs } else { home };
    dir.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_from_the_home_only_when_it_starts_with_one_and_relative_is_from_the_base() {
        assert_eq!(resolve("/etc/hostname", Path::new("/base")), Path::new("/etc/hostname"));
        assert_eq!(resolve("  rel/x.csv ", Path::new("/base")), Path::new("/base/rel/x.csv"));
        assert!(resolve("~/a/b.csv", Path::new("/base")).ends_with("a/b.csv"));
        assert!(!resolve("~/a/b.csv", Path::new("/base")).starts_with("/base"));
    }

    #[test]
    fn the_open_file_is_where_a_save_is_offered_and_otherwise_a_name_in_the_documents() {
        assert_eq!(suggested(Some(Path::new("/x/y.csv")), "sheet.csv"), Path::new("/x/y.csv"));
        assert!(suggested(None, "sheet.csv").ends_with("sheet.csv"));
    }
}
