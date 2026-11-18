//! A walk of a workshop that a sealed command cannot steer (#1124, N2).
//!
//! The shell walks a workshop as the person, in the host namespace, while sealed jobs may still
//! be rewriting it: to put a mark on what is in it (`planted`) and to count what it holds
//! (`sealed_disk`). A walk by path follows a directory that a job has swapped for a symlink
//! between the check and the use, so it would list and mark something outside. This one holds a
//! file descriptor for each directory and opens each child relative to it with `O_NOFOLLOW`, so
//! a link is never entered, and it re-checks the opened object (`fstat`) against what it saw. It
//! does not cross onto another filesystem (`st_dev`), and it goes no deeper than [`MAX_DEPTH`].

use std::ffi::{CStr, CString};
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Deeper than this is left to the location rule.
const MAX_DEPTH: usize = 64;

/// What a visit is told: an open descriptor for a directory or a regular file (opened without
/// following a link), or `-1` for anything else, with its `stat`. It returns `false` to stop.
pub(crate) type Visit<'a> = dyn FnMut(RawFd, &libc::stat) -> bool + 'a;

/// Visit `root` and everything under it, at most `limit` entries below it. How many entries
/// below `root` were seen.
pub(crate) fn walk(root: &Path, limit: usize, visit: &mut Visit) -> usize {
    let Ok(path) = CString::new(root.as_os_str().as_bytes()) else { return 0 };
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe { libc::open(path.as_ptr(), flags) };
    if fd < 0 {
        return 0;
    }
    let mut seen = 0;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } == 0 && visit(fd, &st) {
        descend(fd, st.st_dev, 0, limit, &mut seen, visit);
    }
    unsafe { libc::close(fd) };
    seen
}

/// The names in the directory `dir` (not `.` and `..`).
fn names(dir: RawFd) -> Vec<CString> {
    let dup = unsafe { libc::fcntl(dir, libc::F_DUPFD_CLOEXEC, 3) };
    if dup < 0 {
        return Vec::new();
    }
    let stream = unsafe { libc::fdopendir(dup) };
    if stream.is_null() {
        unsafe { libc::close(dup) };
        return Vec::new();
    }
    let mut out = Vec::new();
    loop {
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes() != b"." && name.to_bytes() != b".." {
            out.push(name.to_owned());
        }
    }
    unsafe { libc::closedir(stream) };
    out
}

/// A directory a job set to mode 000 holds files that still count, and the job (the same account)
/// can undo it any time, so the walk may too: the directory it saw is opened `O_PATH`, checked to
/// be that one (device and inode), given back the owner's access through its descriptor, and
/// opened again (#1124, L4). No link is followed at any step. `-1` when it cannot be done.
fn open_after_chmod(dir: RawFd, name: &CStr, seen: &libc::stat, flags: i32) -> RawFd {
    let held = unsafe { libc::openat(dir, name.as_ptr(), libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if held < 0 {
        return -1;
    }
    let mut now: libc::stat = unsafe { std::mem::zeroed() };
    let mut fd = -1;
    if unsafe { libc::fstat(held, &mut now) } == 0 && now.st_ino == seen.st_ino && now.st_dev == seen.st_dev && now.st_uid == unsafe { libc::geteuid() } {
        let magic = CString::new(format!("/proc/self/fd/{held}")).unwrap_or_default();
        let mode = (now.st_mode & 0o7777) | 0o700;
        if unsafe { libc::chmod(magic.as_ptr(), mode) } == 0 {
            fd = unsafe { libc::openat(dir, name.as_ptr(), flags) };
        }
    }
    unsafe { libc::close(held) };
    fd
}

fn descend(dir: RawFd, dev: libc::dev_t, depth: usize, limit: usize, seen: &mut usize, visit: &mut Visit) -> bool {
    if depth >= MAX_DEPTH {
        return true;
    }
    for name in names(dir) {
        if *seen >= limit {
            return false;
        }
        *seen += 1;
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatat(dir, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 || st.st_dev != dev {
            continue;
        }
        let kind = st.st_mode & libc::S_IFMT;
        let flags = match kind {
            libc::S_IFDIR => libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            libc::S_IFREG => libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            _ => {
                if !visit(-1, &st) {
                    return false;
                }
                continue;
            }
        };
        let mut fd = unsafe { libc::openat(dir, name.as_ptr(), flags) };
        if fd < 0 && kind == libc::S_IFDIR {
            fd = open_after_chmod(dir, &name, &st, flags);
        }
        let mut now: libc::stat = unsafe { std::mem::zeroed() };
        // What opened must be what was seen: same device and inode, same kind. Else it was swapped.
        let same = fd >= 0
            && unsafe { libc::fstat(fd, &mut now) } == 0
            && now.st_dev == dev
            && now.st_ino == st.st_ino
            && now.st_mode & libc::S_IFMT == kind;
        if !same {
            if fd >= 0 {
                unsafe { libc::close(fd) };
            }
            // A file that would not open (no permission) still counts for a sum.
            if fd < 0 && kind == libc::S_IFREG && !visit(-1, &st) {
                return false;
            }
            continue;
        }
        let mut go = visit(fd, &now);
        if go && kind == libc::S_IFDIR {
            go = descend(fd, dev, depth + 1, limit, seen, visit);
        }
        unsafe { libc::close(fd) };
        if !go {
            return false;
        }
    }
    true
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn a_directory_swapped_for_a_link_is_not_entered_and_nothing_outside_is_visited() {
        let base = std::env::temp_dir().join(format!("fdwalk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let outside = base.join("outside");
        let shop = base.join("shop");
        std::fs::create_dir_all(outside.join("deep")).unwrap();
        std::fs::create_dir_all(shop.join("real")).unwrap();
        std::fs::write(outside.join("secret"), "x").unwrap();
        std::fs::write(shop.join("real/a"), "x").unwrap();
        // `d` is a link to a folder outside, as a job that swapped it would leave it.
        std::os::unix::fs::symlink(&outside, shop.join("d")).unwrap();
        let mut dirs = 0;
        let mut files = 0;
        let seen = walk(&shop, 1000, &mut |fd, st| {
            if fd >= 0 && st.st_mode & libc::S_IFMT == libc::S_IFDIR {
                dirs += 1;
            }
            if fd >= 0 && st.st_mode & libc::S_IFMT == libc::S_IFREG {
                files += 1;
            }
            true
        });
        assert_eq!((dirs, files), (2, 1), "the shop and real, and real/a: not outside's");
        assert_eq!(seen, 3, "real, real/a and the link itself");
        // A root that is itself a link is refused.
        assert_eq!(walk(&shop.join("d"), 1000, &mut |_, _| true), 0);
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// L4: a folder a job made unreadable still counts, and is not a way to hide what is in it.
    #[test]
    fn a_folder_set_to_mode_zero_is_counted_all_the_same() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("yantrik-fdwalk-000-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("hidden/inner")).unwrap();
        std::fs::write(root.join("hidden/inner/big"), vec![1u8; 8192]).unwrap();
        std::fs::set_permissions(root.join("hidden"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let mut files = 0;
        let seen = walk(&root, 100, &mut |_, st| {
            if st.st_mode & libc::S_IFMT == libc::S_IFREG {
                files += 1;
            }
            true
        });
        std::fs::set_permissions(root.join("hidden"), std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        // As root, mode 000 stops nothing and the count is the same.
        assert_eq!((files, seen), (1, 3), "hidden, inner and big were all seen");
    }
}
