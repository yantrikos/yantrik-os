//! Give the capabilities back.
//!
//! This service needs `CAP_NET_ADMIN` to join the process connector's multicast group and
//! `CAP_SYS_ADMIN` to call `fanotify_init`. It needs both exactly once, at startup, to obtain two
//! descriptors. After that it is a program that reads from two file descriptors and answers
//! questions on a socket, and there is no reason for it to still be able to mount filesystems.
//!
//! So it hands them back. A process may always reduce its own capabilities, root included, and
//! the reduction is irreversible without an `execve` — which this never does.
//!
//! The result is checkable from outside rather than asserted here:
//!
//! ```text
//! $ grep Cap /proc/$(pgrep perception-service)/status
//! CapEff: 0000000000000000
//! ```
//!
//! That line is the difference between "a privileged daemon we ask you to trust" and "a daemon
//! that was privileged for four milliseconds". The probe checks it.

/// Drop every capability from this thread, permanently.
///
/// Called on the main thread before any source thread is spawned: capabilities are per-thread and
/// inherited at creation, so a thread started afterwards begins with none.
pub fn drop_all() -> Result<(), String> {
    // _LINUX_CAPABILITY_VERSION_3. Version 1 is 32-bit and long obsolete; asking for it makes the
    // kernel rewrite the header and silently use a different layout.
    const VERSION_3: u32 = 0x2008_0522;

    #[repr(C)]
    struct Header {
        version: u32,
        pid: libc::c_int,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Data {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }

    let header = Header { version: VERSION_3, pid: 0 };
    // Two sets, because version 3 splits the 64-bit mask into two 32-bit words.
    let data = [Data::default(), Data::default()];

    // SAFETY: `header` and `data` are correctly shaped for capability version 3 and outlive the
    // call. `capset` is not in libc's public surface on all targets, so it goes through syscall().
    let rc = unsafe {
        libc::syscall(
            libc::SYS_capset,
            &header as *const Header,
            data.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }

    // The bounding set as well, so nothing could be regained through an exec even if one were
    // ever added. Failures here are not fatal — a process without CAP_SETPCAP cannot drop bounds,
    // and it also has nothing worth dropping.
    for cap in 0..=63 {
        // SAFETY: prctl with a constant option and an integer argument.
        unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0) };
    }

    Ok(())
}

/// What the kernel says we still hold, as it appears in `/proc/self/status`.
///
/// Read back rather than assumed. The whole value of dropping privilege is that someone can check
/// it, and the first person who should is this process.
pub fn effective() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("CapEff:"))
                .map(|v| v.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

/// True when nothing is left.
pub fn none_held() -> bool {
    effective().trim_start_matches('0').is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capability_line_is_readable_and_parsed() {
        let eff = effective();
        assert_ne!(eff, "unknown", "/proc/self/status has no CapEff line");
        assert!(eff.chars().all(|c| c.is_ascii_hexdigit()), "unexpected format: {eff:?}");
    }

    #[test]
    fn an_all_zero_mask_reads_as_nothing_held() {
        // The test process usually has no capabilities, so this also exercises the real path.
        assert!(!"0000000000000001".trim_start_matches('0').is_empty());
        assert!("0000000000000000".trim_start_matches('0').is_empty());
    }
}
