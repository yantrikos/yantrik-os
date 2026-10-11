//! Whether our group is a user-private group — the one case a group-writable directory on the
//! way to ours is still ours alone.
//!
//! The test is pam_umask's `usergroups` one: the effective group has the same name as the user.
//! Matching gids alone is not enough — a shared primary group such as `users` has a gid too, and
//! every account in it could write to a 0775 directory of that group.

#[cfg(unix)]
use std::ffi::CStr;

/// Our effective gid, if it is a user-private group for `uid`; `None` otherwise, or if either
/// name cannot be looked up (refusing is the safe answer).
#[cfg(unix)]
pub(super) fn private_group(uid: u32) -> Option<u32> {
    // SAFETY: getegid cannot fail and touches no memory.
    let gid = unsafe { libc::getegid() };
    let user = user_name(uid)?;
    let group = group_name(gid)?;
    (user == group).then_some(gid)
}

/// The login name for `uid`, via the re-entrant `getpwuid_r`.
#[cfg(unix)]
pub(super) fn user_name(uid: u32) -> Option<Vec<u8>> {
    lookup(|buf| {
        // SAFETY: all-zero is a valid `passwd` (null pointers, zero ids); it is only read after
        // getpwuid_r has filled it in.
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: every pointer is to a live local or to `buf`, with `buf`'s true length.
        let rc = unsafe { libc::getpwuid_r(uid, &mut entry, buf.as_mut_ptr(), buf.len(), &mut found) };
        if rc != 0 || found.is_null() {
            return Err(rc);
        }
        // SAFETY: on success pw_name points at a NUL-terminated string inside `buf`, which is
        // still alive here.
        Ok(unsafe { CStr::from_ptr(entry.pw_name) }.to_bytes().to_vec())
    })
}

/// The name of group `gid`, via the re-entrant `getgrgid_r`.
#[cfg(unix)]
pub(super) fn group_name(gid: u32) -> Option<Vec<u8>> {
    lookup(|buf| {
        // SAFETY: as in `user_name`, for `group`.
        let mut entry: libc::group = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::group = std::ptr::null_mut();
        // SAFETY: every pointer is to a live local or to `buf`, with `buf`'s true length.
        let rc = unsafe { libc::getgrgid_r(gid, &mut entry, buf.as_mut_ptr(), buf.len(), &mut found) };
        if rc != 0 || found.is_null() {
            return Err(rc);
        }
        // SAFETY: on success gr_name points at a NUL-terminated string inside `buf`.
        Ok(unsafe { CStr::from_ptr(entry.gr_name) }.to_bytes().to_vec())
    })
}

/// Run a `get*_r` lookup, growing the buffer while it answers ERANGE (a group with many members
/// can need more than the first guess). Any other failure, or no such entry, is `None`.
#[cfg(unix)]
fn lookup(mut call: impl FnMut(&mut [libc::c_char]) -> Result<Vec<u8>, i32>) -> Option<Vec<u8>> {
    let mut size = 1024;
    while size <= 1 << 20 {
        let mut buf = vec![0 as libc::c_char; size];
        match call(&mut buf) {
            Ok(name) => return Some(name),
            Err(libc::ERANGE) => size *= 4,
            Err(_) => return None,
        }
    }
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn root_is_named_root_in_both_databases() {
        // Every Linux this runs on has uid 0 and gid 0 named root.
        assert_eq!(user_name(0).as_deref(), Some(&b"root"[..]));
        assert_eq!(group_name(0).as_deref(), Some(&b"root"[..]));
    }

    #[test]
    fn our_group_counts_as_private_exactly_when_the_names_match() {
        // SAFETY: geteuid/getegid cannot fail and touch no memory.
        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
        let names_match = matches!((user_name(uid), group_name(gid)), (Some(u), Some(g)) if u == g);
        assert_eq!(private_group(uid), names_match.then_some(gid));
    }

    #[test]
    fn an_id_with_no_entry_has_no_name() {
        assert_eq!(user_name(u32::MAX - 7), None);
        assert_eq!(group_name(u32::MAX - 7), None);
    }
}
