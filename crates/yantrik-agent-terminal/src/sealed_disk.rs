//! What a sealed command may leave on the person's disk (#1124, L1).
//!
//! The systemd scope holds CPU, memory, tasks and time, and has no disk quota. A six-hour build
//! that writes without end would fill the person's `/home`. So while a sealed job runs, its worker
//! looks at two numbers every few seconds and stops the job when either is crossed:
//!
//! - what the workshop holds (allocated blocks, links not followed), against [`Caps::disk_max`];
//! - what the filesystem under it has left, against [`Caps::free_min`].
//!
//! - how many entries it holds and how many inodes the filesystem has left (a loop of empty files
//!   costs no blocks).
//!
//! The walk stops as soon as it is over the bound, so a runaway folder costs one bounded sweep. A
//! job is also refused at the start when the workshop is already over, or the disk already short.

use std::path::Path;

use crate::sealed::Caps;

/// How often the worker looks.
pub(crate) const EVERY: std::time::Duration = std::time::Duration::from_secs(8);

/// The most entries (files and folders) a workshop may hold, and the fewest inodes the filesystem
/// may be left with. A loop of empty files costs no blocks and would fill `/home`'s inodes
/// without crossing either byte bound (#1124, N5).
pub(crate) const MAX_ENTRIES: usize = 5_000_000;
pub(crate) const MIN_FREE_INODES: u64 = 100_000;

/// What a workshop holds.
pub(crate) struct Held {
    /// Allocated bytes, a hard-linked file counted once.
    pub bytes: u64,
    pub entries: usize,
}

/// What is under `root`, not following links (the walk is by descriptor, `fdwalk`, so a link a job
/// swaps in is not entered, and it does not cross onto another filesystem), stopping once the sum
/// passes `stop_after` or the entries pass [`MAX_ENTRIES`].
pub(crate) fn usage(root: &Path, stop_after: u64) -> Held {
    let mut bytes = 0u64;
    let mut seen_links = std::collections::HashSet::new();
    let entries = crate::fdwalk::walk(root, MAX_ENTRIES + 1, &mut |_, st| {
        if st.st_nlink > 1 && st.st_mode & libc::S_IFMT != libc::S_IFDIR && !seen_links.insert((st.st_dev, st.st_ino)) {
            return true;
        }
        bytes += (st.st_blocks as u64) * 512;
        bytes <= stop_after
    });
    Held { bytes, entries }
}

/// Bytes available to the person's account on the filesystem holding `path`, and its free inodes
/// (`None` where the filesystem has no inode count, as btrfs does not).
pub(crate) fn free_space(path: &Path) -> Option<(u64, Option<u64>)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let inodes = (st.f_files != 0).then_some(st.f_favail as u64);
    Some(((st.f_bavail as u64).saturating_mul(st.f_frsize as u64), inodes))
}

/// Why a job in `workshop` should not run (or go on), if it should not.
pub(crate) fn verdict(workshop: &Path, caps: &Caps) -> Option<String> {
    if let Some((free, inodes)) = free_space(workshop) {
        if free < caps.free_min {
            return Some(format!(
                "the disk under the workshop has {} MiB free, below the {} MiB a sealed command must leave",
                free >> 20,
                caps.free_min >> 20
            ));
        }
        if inodes.is_some_and(|n| n < MIN_FREE_INODES) {
            return Some("the disk under the workshop is nearly out of file slots (inodes)".to_string());
        }
    }
    let held = usage(workshop, caps.disk_max);
    if held.entries > MAX_ENTRIES {
        return Some(format!("the workshop holds more than {MAX_ENTRIES} files and folders, the most a sealed command may leave in it"));
    }
    (held.bytes > caps.disk_max).then(|| {
        format!("the workshop holds more than {} GiB, the most a sealed command may leave in it", caps.disk_max >> 30)
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sealed-disk-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        d
    }

    #[test]
    fn usage_counts_blocks_once_per_link_and_does_not_follow_symlinks() {
        let d = dir("usage");
        std::fs::write(d.join("sub/a"), vec![1u8; 64 * 1024]).unwrap();
        std::fs::hard_link(d.join("sub/a"), d.join("b")).unwrap();
        std::os::unix::fs::symlink("/usr", d.join("link")).unwrap();
        let held = usage(&d, u64::MAX);
        assert!(held.bytes >= 64 * 1024 && held.bytes < 200 * 1024, "{}", held.bytes);
        assert_eq!(held.entries, 4, "sub, sub/a, b and the link");
        // Stops early once over.
        assert!(usage(&d, 1024).bytes > 1024);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_workshop_over_its_bound_or_a_disk_short_of_free_space_stops_the_job() {
        let d = dir("verdict");
        std::fs::write(d.join("big"), vec![1u8; 256 * 1024]).unwrap();
        let mut caps = Caps::default();
        assert_eq!(verdict(&d, &caps), None);
        caps.disk_max = 100 * 1024;
        assert!(verdict(&d, &caps).unwrap().contains("workshop holds"));
        let mut caps = Caps::default();
        caps.free_min = u64::MAX;
        assert!(verdict(&d, &caps).unwrap().contains("free"));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
