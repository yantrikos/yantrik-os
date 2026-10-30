//! Putting something away without deleting it.
//!
//! "Archive" on the desktop is a move to `~/Archive/<YYYY-MM-DD>/`, and the toast that says so
//! carries an Undo that moves it back. There is no delete anywhere in this module, and a test
//! reads the source to keep it that way. Two things make the move safe to offer to a mind as
//! well as to a person:
//!
//! - the name must be one thing directly on the desktop ([`crate::items::is_plain_name`]), and
//!   what is moved is that entry itself: a link is moved as a link and never followed;
//! - the move never lands on top of anything. It is `renameat2(RENAME_NOREPLACE)`, so a name
//!   already in the archive gets ` (2)` and the earlier file is untouched, even if it appeared
//!   a moment ago. A move across file systems is refused rather than made as copy-then-delete,
//!   because that would be a delete.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use crate::items::is_plain_name;

/// How many `name (n)` are tried before giving up. A person has not archived a hundred files
/// of one name on one day.
const TRIES: u32 = 100;

/// What was moved, so it can be moved back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archived {
    /// Where it was, on the desktop.
    pub from: PathBuf,
    /// Where it is now.
    pub to: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("`{0}` is not the name of one thing on the desktop")]
    BadName(String),
    #[error("the date `{0}` is not YYYY-MM-DD")]
    BadDate(String),
    #[error("there is nothing called `{0}` on the desktop")]
    NotThere(String),
    #[error("the archive is on another disk from the desktop, so it cannot be moved without copying; nothing was moved")]
    OtherDisk,
    #[error("no free name was found in the archive for `{0}`")]
    NoName(String),
    #[error("{0}: {1}")]
    Io(&'static str, std::io::Error),
}

/// Move `name` from `desktop` into `archive_root/<date>/`.
pub fn archive(desktop: &Path, name: &str, archive_root: &Path, date: &str) -> Result<Archived, ArchiveError> {
    if !is_plain_name(name) {
        return Err(ArchiveError::BadName(name.to_string()));
    }
    if !is_date(date) {
        return Err(ArchiveError::BadDate(date.to_string()));
    }
    let from = desktop.join(name);
    match std::fs::symlink_metadata(&from) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ArchiveError::NotThere(name.to_string())),
        Err(e) => return Err(ArchiveError::Io("the desktop entry cannot be read", e)),
    }
    let day = archive_root.join(date);
    // 0700: what a person put away is theirs, whoever else can read the home folder.
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&day)
        .map_err(|e| ArchiveError::Io("the archive folder cannot be made", e))?;
    let to = move_without_replacing(&from, &day, name)?;
    Ok(Archived { from, to })
}

/// Move it back. If something has taken its name on the desktop since, it comes back as
/// `name (2)` instead of replacing that. Returns where it is now.
pub fn undo(done: &Archived) -> Result<PathBuf, ArchiveError> {
    let dir = done.from.parent().unwrap_or_else(|| Path::new("."));
    let name = done.from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    move_without_replacing(&done.to, dir, &name)
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

/// `name`, `name (2)`, `name (3)` … in `dir`, whichever the move can take without replacing.
fn move_without_replacing(from: &Path, dir: &Path, name: &str) -> Result<PathBuf, ArchiveError> {
    for n in 1..=TRIES {
        let to = dir.join(numbered(name, n));
        match rename_no_replace(from, &to) {
            Ok(()) => return Ok(to),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => return Err(ArchiveError::OtherDisk),
            Err(e) => return Err(ArchiveError::Io("the move failed", e)),
        }
    }
    Err(ArchiveError::NoName(name.to_string()))
}

/// `report.pdf` → `report (2).pdf`; `folder` → `folder (2)`; `.profile` → `.profile (2)`.
fn numbered(name: &str, n: u32) -> String {
    if n == 1 {
        return name.to_string();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({n}).{ext}"),
        _ => format!("{name} ({n})"),
    }
}

/// A rename that fails with `AlreadyExists` instead of replacing what is at `to`.
fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    let (a, b) = (c_path(from)?, c_path(to)?);
    // SAFETY: both are NUL-terminated strings that outlive the call.
    let rc = unsafe { libc::renameat2(libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), libc::RENAME_NOREPLACE) };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        // The file system cannot do an atomic no-replace rename (some network and FUSE ones):
        // check first, and say so, since the check and the move are two steps there.
        Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::EOPNOTSUPP) => {
            tracing::debug!("renameat2 is not supported here; checking before the move");
            if std::fs::symlink_metadata(to).is_ok() {
                return Err(std::io::ErrorKind::AlreadyExists.into());
            }
            std::fs::rename(from, to)
        }
        _ => Err(err),
    }
}

fn c_path(p: &Path) -> std::io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| std::io::ErrorKind::InvalidInput.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Home {
        _tmp: tempfile::TempDir,
        desktop: PathBuf,
        archive: PathBuf,
    }

    fn home() -> Home {
        let tmp = tempfile::tempdir().unwrap();
        let desktop = tmp.path().join("Desktop");
        std::fs::create_dir(&desktop).unwrap();
        let archive = tmp.path().join("Archive");
        Home { desktop, archive, _tmp: tmp }
    }

    #[test]
    fn archiving_moves_the_file_with_its_contents_and_deletes_nothing() {
        let h = home();
        std::fs::write(h.desktop.join("Report.pdf"), "the report").unwrap();
        let done = archive(&h.desktop, "Report.pdf", &h.archive, "2026-10-30").unwrap();
        assert_eq!(done.to, h.archive.join("2026-10-30/Report.pdf"));
        assert!(!h.desktop.join("Report.pdf").exists());
        assert_eq!(std::fs::read_to_string(&done.to).unwrap(), "the report");
    }

    #[test]
    fn undo_puts_it_back_where_it_was() {
        let h = home();
        std::fs::write(h.desktop.join("a.txt"), "A").unwrap();
        let done = archive(&h.desktop, "a.txt", &h.archive, "2026-10-30").unwrap();
        let back = undo(&done).unwrap();
        assert_eq!(back, h.desktop.join("a.txt"));
        assert_eq!(std::fs::read_to_string(&back).unwrap(), "A");
        assert!(!done.to.exists());
    }

    #[test]
    fn undo_does_not_replace_what_took_its_name_meanwhile() {
        let h = home();
        std::fs::write(h.desktop.join("a.txt"), "old").unwrap();
        let done = archive(&h.desktop, "a.txt", &h.archive, "2026-10-30").unwrap();
        std::fs::write(h.desktop.join("a.txt"), "new one").unwrap();
        let back = undo(&done).unwrap();
        assert_eq!(back, h.desktop.join("a (2).txt"));
        assert_eq!(std::fs::read_to_string(h.desktop.join("a.txt")).unwrap(), "new one");
        assert_eq!(std::fs::read_to_string(back).unwrap(), "old");
    }

    #[test]
    fn a_second_file_of_the_same_name_the_same_day_does_not_replace_the_first() {
        let h = home();
        for body in ["first", "second", "third"] {
            std::fs::write(h.desktop.join("notes"), body).unwrap();
            archive(&h.desktop, "notes", &h.archive, "2026-10-30").unwrap();
        }
        let day = h.archive.join("2026-10-30");
        assert_eq!(std::fs::read_to_string(day.join("notes")).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(day.join("notes (2)")).unwrap(), "second");
        assert_eq!(std::fs::read_to_string(day.join("notes (3)")).unwrap(), "third");
    }

    #[test]
    fn a_folder_goes_whole_and_a_link_goes_as_a_link() {
        let h = home();
        std::fs::create_dir(h.desktop.join("Starfall")).unwrap();
        std::fs::write(h.desktop.join("Starfall/inner.txt"), "in").unwrap();
        let target = h.desktop.parent().unwrap().join("elsewhere.txt");
        std::fs::write(&target, "elsewhere").unwrap();
        std::os::unix::fs::symlink(&target, h.desktop.join("Shortcut")).unwrap();
        let dir = archive(&h.desktop, "Starfall", &h.archive, "2026-10-30").unwrap();
        assert_eq!(std::fs::read_to_string(dir.to.join("inner.txt")).unwrap(), "in");
        let link = archive(&h.desktop, "Shortcut", &h.archive, "2026-10-30").unwrap();
        assert!(std::fs::symlink_metadata(&link.to).unwrap().file_type().is_symlink(), "moved as a link");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "elsewhere", "what it pointed at is untouched");
    }

    #[test]
    fn a_name_that_reaches_out_of_the_desktop_is_refused_and_nothing_moves() {
        let h = home();
        let outside = h.desktop.parent().unwrap().join("secret.txt");
        std::fs::write(&outside, "secret").unwrap();
        for bad in ["../secret.txt", "/etc/hostname", "..", ".", "", "a/../../secret.txt"] {
            assert!(matches!(archive(&h.desktop, bad, &h.archive, "2026-10-30"), Err(ArchiveError::BadName(_))), "{bad:?}");
        }
        assert!(outside.exists());
        assert!(!h.archive.exists(), "not even the archive folder was made");
    }

    #[test]
    fn a_date_that_is_not_a_date_is_refused() {
        let h = home();
        std::fs::write(h.desktop.join("a"), "").unwrap();
        for bad in ["", "../..", "2026-10-3", "2026/10/30", "20261030xx"] {
            assert!(matches!(archive(&h.desktop, "a", &h.archive, bad), Err(ArchiveError::BadDate(_))), "{bad:?}");
        }
        assert!(h.desktop.join("a").exists());
    }

    #[test]
    fn something_that_is_not_there_is_said_so() {
        let h = home();
        assert!(matches!(archive(&h.desktop, "ghost", &h.archive, "2026-10-30"), Err(ArchiveError::NotThere(_))));
    }

    #[test]
    fn the_archive_folder_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let h = home();
        std::fs::write(h.desktop.join("a"), "").unwrap();
        archive(&h.desktop, "a", &h.archive, "2026-10-30").unwrap();
        let mode = std::fs::metadata(h.archive.join("2026-10-30")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    /// The point of the module, kept by reading it: nothing here removes anything.
    #[test]
    fn this_module_has_no_delete_in_it() {
        let src = include_str!("archive.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for word in ["remove_file", "remove_dir", "unlink", "fs::copy", "truncate"] {
            assert!(!code.contains(word), "`{word}` in archive.rs");
        }
    }

    #[test]
    fn numbering_keeps_the_extension_and_leaves_dot_files_alone() {
        assert_eq!(numbered("report.pdf", 2), "report (2).pdf");
        assert_eq!(numbered("folder", 3), "folder (3)");
        assert_eq!(numbered(".profile", 2), ".profile (2)");
        assert_eq!(numbered("a.tar.gz", 2), "a.tar (2).gz");
    }
}
