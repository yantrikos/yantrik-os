//! Opening a file or directory so that what is examined is what is read.
//!
//! A check made on a path (`stat`, then `open`) can be answered by one file and acted on by
//! another if something swaps a link in between. These helpers open first, refusing a link at
//! the last component (`O_NOFOLLOW`), and hand back the descriptor: the caller asks `fstat` of the
//! descriptor and reads from it, so the file looked at is the file read. `O_NONBLOCK` keeps a
//! planted FIFO from holding the open, and `O_CLOEXEC` keeps the descriptor from a child.

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Why a path was not opened.
#[derive(Debug)]
pub enum Refusal {
    /// Nothing is there.
    Absent,
    /// The last component is a symbolic link.
    Link,
    /// Something else went wrong.
    Failed(io::Error),
}

impl Refusal {
    fn of(e: io::Error) -> Refusal {
        match e.raw_os_error() {
            Some(libc::ENOENT) => Refusal::Absent,
            // ELOOP is what O_NOFOLLOW answers for a symlink in the last component. (A directory
            // open that meets a link answers ENOTDIR instead, and is refused as a failure.)
            Some(libc::ELOOP) => Refusal::Link,
            _ => Refusal::Failed(e),
        }
    }
}

const FILE_FLAGS: i32 = libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;

/// Open `path` for reading, never through a link.
pub fn open_file(path: &Path) -> Result<File, Refusal> {
    std::fs::OpenOptions::new().read(true).custom_flags(FILE_FLAGS).open(path).map_err(Refusal::of)
}

/// Open the directory at `path`, never through a link, for `fstat` and for [`open_file_in`].
pub fn open_dir(path: &Path) -> Result<File, Refusal> {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(Refusal::of)
}

/// Open `name` (one path component) inside the already-open directory `dir`, never through a
/// link: the file is looked up in the directory that was examined, not in whatever the path now
/// names.
pub fn open_file_in(dir: &File, name: &str) -> Result<File, Refusal> {
    let name = CString::new(Path::new(name).as_os_str().as_bytes()).map_err(|e| Refusal::Failed(io::Error::other(e)))?;
    // SAFETY: `dir` is an open descriptor for the duration of the call and `name` is a
    // NUL-terminated string that outlives it.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | FILE_FLAGS) };
    if fd < 0 {
        return Err(Refusal::of(io::Error::last_os_error()));
    }
    // SAFETY: `fd` was just returned by a successful openat and nothing else owns it.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::os::unix::fs::symlink;

    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-nofollow-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_plain_file_opens_and_reads_through_the_descriptor() {
        let dir = scratch("plain");
        std::fs::write(dir.join("f"), "on\n").unwrap();
        let mut text = String::new();
        open_file(&dir.join("f")).unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "on\n");
        let held = open_dir(&dir).unwrap();
        let mut again = String::new();
        open_file_in(&held, "f").unwrap().read_to_string(&mut again).unwrap();
        assert_eq!(again, "on\n");
    }

    #[test]
    fn a_link_is_refused_at_the_last_component_and_nothing_is_absent_not_a_failure() {
        let dir = scratch("link");
        std::fs::write(dir.join("real"), "on\n").unwrap();
        symlink(dir.join("real"), dir.join("file-link")).unwrap();
        symlink(&dir, dir.join("dir-link")).unwrap();
        assert!(matches!(open_file(&dir.join("file-link")), Err(Refusal::Link)));
        assert!(matches!(open_dir(&dir.join("dir-link")), Err(Refusal::Failed(_))), "a linked directory is not examined");
        assert!(matches!(open_file_in(&open_dir(&dir).unwrap(), "file-link"), Err(Refusal::Link)));
        assert!(matches!(open_file(&dir.join("nothing")), Err(Refusal::Absent)));
        assert!(matches!(open_dir(&dir.join("nothing")), Err(Refusal::Absent)));
        assert!(open_dir(&dir.join("real")).is_err(), "a file is not a directory");
    }

    #[test]
    fn the_file_read_is_the_file_examined_even_if_the_path_is_swapped_afterwards() {
        let dir = scratch("swap");
        std::fs::write(dir.join("f"), "on\n").unwrap();
        let held = open_file(&dir.join("f")).unwrap();
        std::fs::remove_file(dir.join("f")).unwrap();
        std::fs::write(dir.join("f"), "off\n").unwrap();
        let mut text = String::new();
        (&held).read_to_string(&mut text).unwrap();
        assert_eq!(text, "on\n", "the descriptor still names the file that was opened");
    }
}
