//! The disk-space guard for recordings.
//!
//! A screen recording grows for as long as nobody stops it, and a disk that fills mid-recording
//! takes the session's other writes with it: settings, the notification store, the memory. So a
//! recording does not start with under 1 GB free, and one that is running stops cleanly — the
//! file finished and playable — when free space falls under it, and says why.

use std::path::Path;

/// Below this, no recording runs.
pub const MIN_FREE: u64 = 1 << 30;

/// Bytes free to this account on the filesystem holding `path`, or `None` when it cannot be
/// read.
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a valid NUL-terminated path and a zeroed struct the call fills.
    if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

/// Whether there is room to record. Unknown counts as room: statvfs failing on the home is a
/// broken machine, and refusing to record there would be the least of its problems; the watch
/// asks again every few seconds anyway.
pub fn room(free: Option<u64>) -> bool {
    free.is_none_or(|f| f >= MIN_FREE)
}

/// "1.2 GB", for the sentence that says why a recording stopped.
pub fn words(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_one_gigabyte_free_there_is_no_room_to_record() {
        assert!(!room(Some(MIN_FREE - 1)));
        assert!(!room(Some(0)));
        assert!(room(Some(MIN_FREE)));
        assert!(room(Some(40 << 30)));
        assert!(room(None), "an unreadable disk is not a reason to refuse");
        assert_eq!(words(MIN_FREE), "1.0 GB");
    }

    #[test]
    fn the_free_space_of_a_real_directory_can_be_read() {
        assert!(free_bytes(&std::env::temp_dir()).is_some());
        assert!(free_bytes(Path::new("/no/such/place/at/all")).is_none());
    }
}
