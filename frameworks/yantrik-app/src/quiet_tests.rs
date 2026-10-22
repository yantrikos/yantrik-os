//! The quiet read against everything that may be at a path: what it reads, what it refuses, and
//! that a refusal reads none of the thing's bytes and never waits. The bytes read are this
//! thread's own count (`/proc/thread-self/io`, `rchar`), so tests running beside it do not count.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;

/// A sentinel file larger than any limit here, so reading any of it shows in the count.
pub(crate) const BIG: usize = 4 * 1024 * 1024;

pub(crate) fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yantrik-quiet-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The bytes this thread has read so far, by the kernel's count.
#[cfg(target_os = "linux")]
pub(crate) fn read_so_far() -> u64 {
    let io = std::fs::read_to_string("/proc/thread-self/io").expect("/proc/thread-self/io");
    io.lines()
        .find_map(|l| l.strip_prefix("rchar:"))
        .and_then(|n| n.trim().parse().ok())
        .expect("rchar")
}

/// What `f` read, by the kernel's count: the count's own read is a few hundred bytes.
#[cfg(target_os = "linux")]
pub(crate) fn bytes_read_by<R>(f: impl FnOnce() -> R) -> (R, u64) {
    let before = read_so_far();
    let r = f();
    (r, read_so_far().saturating_sub(before))
}

/// Well under any sentinel: the count's own reads, and nothing of the thing looked at.
pub(crate) const NOTHING_OF_IT: u64 = 16 * 1024;

#[cfg(unix)]
pub(crate) fn fifo(path: &Path) {
    use std::os::unix::ffi::OsStrExt as _;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: a valid NUL-terminated path, read by the call only.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0, "mkfifo {}", path.display());
}

/// `f`, on a thread of its own, within `deadline`: a read that blocks fails the test, not hangs it.
pub(crate) fn within<R: Send + 'static>(deadline: Duration, f: impl FnOnce() -> R + Send + 'static) -> R {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(deadline).expect("it blocked")
}

/// The things that are not a settings file, each beside a sentinel it must not read.
#[cfg(unix)]
pub(crate) fn hostile(d: &Path) -> Vec<(&'static str, PathBuf)> {
    let secret = d.join("secret.json");
    std::fs::write(&secret, "S".repeat(BIG)).unwrap();
    let pipe = d.join("pipe");
    fifo(&pipe);
    let link = |name: &str, to: &Path| {
        let at = d.join(name);
        std::os::unix::fs::symlink(to, &at).unwrap();
        at
    };
    let big = d.join("big.json");
    std::fs::write(&big, "B".repeat(BIG)).unwrap();
    vec![
        ("a link to a sentinel file", link("to-secret", &secret)),
        ("a FIFO", pipe.clone()),
        ("a link to a FIFO", link("to-pipe", &pipe)),
        ("a link to /dev/zero", link("to-zero", Path::new("/dev/zero"))),
        ("/dev/zero itself", PathBuf::from("/dev/zero")),
        ("a directory", d.to_path_buf()),
        ("a file over the limit", big),
    ]
}

#[test]
fn a_small_regular_file_is_read_and_nothing_is_missing() {
    let d = dir("plain");
    let file = d.join("a.json");
    std::fs::write(&file, b"{\"a\":1}").unwrap();
    assert_eq!(read_within(&file, 1024).unwrap(), Looked::Read(b"{\"a\":1}".to_vec()));
    assert_eq!(read_within(&d.join("none"), 1024).unwrap(), Looked::Missing);
}

#[cfg(target_os = "linux")]
#[test]
fn anything_else_is_refused_by_its_metadata_without_a_byte_of_it_read_or_a_wait() {
    let d = dir("hostile");
    let things = hostile(&d);
    // The count sees a read: following the link the ordinary way reads all of the sentinel.
    let (_, followed) = bytes_read_by(|| std::fs::read(d.join("to-secret")).unwrap());
    assert!(followed >= BIG as u64, "the count is live: {followed}");
    for (what, path) in things {
        let (looked, read) = within(Duration::from_secs(5), move || bytes_read_by(|| read_within(&path, 1024 * 1024).unwrap()));
        assert!(matches!(looked, Looked::Refused(_)), "{what}: {looked:?}");
        assert!(read < NOTHING_OF_IT, "{what}: {read} bytes read");
    }
}

#[cfg(unix)]
#[test]
fn a_link_changed_to_point_elsewhere_is_seen_by_its_metadata() {
    let d = dir("relink");
    let at = d.join("settings.json");
    std::os::unix::fs::symlink("/dev/zero", &at).unwrap();
    let Looked::Refused(first) = read_within(&at, 1024).unwrap() else { panic!("refused") };
    std::fs::remove_file(&at).unwrap();
    std::os::unix::fs::symlink("/dev/null", &at).unwrap();
    let Looked::Refused(second) = read_within(&at, 1024).unwrap() else { panic!("refused") };
    assert_ne!(first, second, "a new link is a new thing at the path");
}
