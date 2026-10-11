//! Whether a program is installed: one answer for the whole shell.
//!
//! The harness catalogue and the accounts each used to look for programs their own way. The
//! harnesses looked on `PATH` and in `~/.local/bin`. The accounts looked on `PATH` split with
//! `split_paths`, where an empty entry means the working directory, and in the directories a
//! per-person npm or bun installs to. So the same `claude` could be found by one and not the
//! other. Both ask here now.

use std::path::{Path, PathBuf};

/// Where installers that need no root put their command, which a session's `PATH` usually lacks:
/// a session manager does not read `~/.profile`.
const PERSONAL_BINS: [&str; 4] = [".npm-global/bin", ".local/node/bin", ".bun/bin", ".local/bin"];

/// The directories to look in: `path`'s entries, then the personal ones it did not already name.
/// Empty entries are dropped: they mean the working directory, and a program "found" in
/// whatever directory the shell was started from is not installed.
pub fn search_dirs(path: &str, home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = path.split(':').filter(|p| !p.is_empty()).map(PathBuf::from).collect();
    if !home.as_os_str().is_empty() {
        for extra in PERSONAL_BINS {
            let dir = home.join(extra);
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// Whether `binary` is an executable file in any of `dirs`.
pub fn found(dirs: &[PathBuf], binary: &str) -> bool {
    !binary.is_empty() && !binary.contains('/') && dirs.iter().any(|d| is_program(&d.join(binary)))
}

/// An executable regular file.
pub fn is_program(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_entries_are_not_the_working_directory() {
        let dirs = search_dirs("/usr/bin::/bin:", Path::new("/home/p"));
        assert!(!dirs.iter().any(|d| d.as_os_str().is_empty()), "{dirs:?}");
        assert_eq!(&dirs[..2], [PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
    }

    #[test]
    fn personal_directories_are_searched_once_and_last_is_local_bin() {
        let dirs = search_dirs("/usr/bin:/home/p/.local/bin", Path::new("/home/p"));
        assert_eq!(dirs.iter().filter(|d| d.ends_with(".local/bin")).count(), 1);
        assert!(dirs.contains(&PathBuf::from("/home/p/.npm-global/bin")));
        assert!(dirs.contains(&PathBuf::from("/home/p/.bun/bin")));
    }

    #[test]
    fn a_name_with_a_slash_or_no_name_is_never_found() {
        let dirs = vec![PathBuf::from("/usr/bin")];
        assert!(!found(&dirs, ""));
        assert!(!found(&dirs, "../bin/sh"));
    }

    #[cfg(unix)]
    #[test]
    fn only_an_executable_file_counts() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("yantrik-programs-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("tool"), "#!/bin/sh\n").unwrap();
        std::fs::write(d.join("data"), "x").unwrap();
        std::fs::set_permissions(d.join("tool"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(d.join("data"), std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::create_dir_all(d.join("dir")).unwrap();
        let dirs = vec![d.clone()];
        assert!(found(&dirs, "tool"));
        assert!(!found(&dirs, "data"), "not executable");
        assert!(!found(&dirs, "dir"), "a directory");
        let _ = std::fs::remove_dir_all(&d);
    }
}
