//! What a shell is given to start from, and what it is not.
//!
//! A shell must not inherit what the app happens to be holding or ignoring: a file descriptor a
//! library opened without close-on-exec (Mesa's `/dev/udmabuf` and memfd after the first frame), a
//! signal the app was started with ignored (a launch from a non-interactive `&` ignores SIGINT
//! and SIGQUIT), or a blocked signal. [`tidy`] runs in the child between the fork and the exec and
//! gives the shell the defaults, whatever the app was started with. It makes no assumption about
//! other libraries' flags.

/// How many descriptors the fallback walks at most.
const WALK_AT_MOST: libc::c_int = 1 << 20;

/// The soft limit on open descriptors, read before the fork: the fallback below must not look it up
/// in the child.
pub fn descriptor_limit() -> libc::c_int {
    let mut limit = libc::rlimit { rlim_cur: 1024, rlim_max: 1024 };
    // SAFETY: `limit` is a live `rlimit`.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return 1024;
    }
    limit.rlim_cur.min(WALK_AT_MOST as libc::rlim_t) as libc::c_int
}

/// Runs in the child between the fork and the exec, so it calls only async-signal-safe functions
/// and allocates nothing.
///
/// - Every signal but SIGKILL and SIGSTOP goes back to its default, and none is blocked.
/// - Every descriptor above the standard three is marked close-on-exec rather than closed, so the
///   exec closes it and std's own pipe, which reports a failed exec, still works. A kernel without
///   `close_range` (ENOSYS) is walked descriptor by descriptor up to `limit`.
///
/// # Safety
/// Only to be called from `pre_exec`.
pub unsafe fn tidy(limit: libc::c_int) {
    const CLOSE_RANGE_CLOEXEC: libc::c_uint = 4;
    for signal in 1..=31 {
        if signal != libc::SIGKILL && signal != libc::SIGSTOP {
            libc::signal(signal, libc::SIG_DFL);
        }
    }
    let mut none: libc::sigset_t = std::mem::zeroed();
    libc::sigemptyset(&mut none);
    libc::sigprocmask(libc::SIG_SETMASK, &none, std::ptr::null_mut());
    if libc::syscall(libc::SYS_close_range, 3 as libc::c_uint, !0 as libc::c_uint, CLOSE_RANGE_CLOEXEC) != 0 {
        for fd in 3..limit {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
}
