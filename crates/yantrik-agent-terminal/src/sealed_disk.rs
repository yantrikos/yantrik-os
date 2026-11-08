//! What a sealed command may leave on the person's disk (#1124, L1).
//!
//! The systemd scope holds CPU, memory, tasks and time, and has no disk quota. A six-hour build
//! that writes without end would fill the person's `/home`. So while a sealed job runs, its worker
//! looks at two numbers every few seconds and stops the job when either is crossed:
//!
//! - what the workshop holds (allocated blocks, links not followed), against [`Caps::disk_max`];
//! - what the filesystem under it has left, against [`Caps::free_min`].
//!
//! The walk stops as soon as it is over the bound, so a runaway folder costs one bounded sweep. A
//! job is also refused at the start when the workshop is already over, or the disk already short.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::sealed::Caps;

/// How often the worker looks.
pub(crate) const EVERY: std::time::Duration = std::time::Duration::from_secs(8);

/// Allocated bytes under `root`, not following links, counting a hard-linked file once, and
/// stopping once the sum passes `stop_after`.
pub(crate) fn usage(root: &Path, stop_after: u64) -> u64 {
    let mut total = 0u64;
    let mut seen_links = std::collections::HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                total += meta.blocks() * 512;
                stack.push(entry.path());
            } else if meta.nlink() > 1 && !seen_links.insert((meta.dev(), meta.ino())) {
                continue;
            } else {
                total += meta.blocks() * 512;
            }
            if total > stop_after {
                return total;
            }
        }
    }
    total
}

/// Bytes available to the person's account on the filesystem holding `path`.
pub(crate) fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0).then(|| (st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
}

/// Why a job in `workshop` should not run (or go on), if it should not.
pub(crate) fn verdict(workshop: &Path, caps: &Caps) -> Option<String> {
    if let Some(free) = free_bytes(workshop) {
        if free < caps.free_min {
            return Some(format!(
                "the disk under the workshop has {} MiB free, below the {} MiB a sealed command must leave",
                free >> 20,
                caps.free_min >> 20
            ));
        }
    }
    let held = usage(workshop, caps.disk_max);
    (held > caps.disk_max).then(|| {
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
        assert!(held >= 64 * 1024 && held < 200 * 1024, "{held}");
        // Stops early once over.
        assert!(usage(&d, 1024) > 1024);
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
