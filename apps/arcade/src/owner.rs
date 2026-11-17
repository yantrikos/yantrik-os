//! Who owns a temporary folder, and whether that owner is still the same process.
//!
//! A bare pid is not an identity: once the owner dies and is reaped its pid can be given to any
//! new process, and `kill(pid, 0)` then says "alive" for a folder nobody owns (the folder is never
//! swept, and in a busy test run the sweep assertion fails). So the record is `<pid> <start>`,
//! where `<start>` is the process's start time in clock ticks since boot (`/proc/<pid>/stat`,
//! field 22). Same pid with another start time is a different process, so the owner is gone.
//! Where the start time cannot be read (no `/proc`, or an old record that is only a pid) the check
//! falls back to the pid alone, which can only err on the side of keeping a folder.

/// The start time of `pid`, if `/proc` tells.
pub fn start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is in parentheses and may hold spaces: fields count from after the last `)`.
    // After it: state is field 3, so start time (field 22) is the 20th token.
    stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
}

/// The record for `pid`: `"<pid> <start>"`, or just `"<pid>"` when the start time is unknown.
pub fn record_for(pid: u32) -> String {
    match start_time(pid) {
        Some(start) => format!("{pid} {start}"),
        None => pid.to_string(),
    }
}

/// True when the process a record names is no longer running: the pid is gone, or it now belongs to
/// a process that started at another time. A record that does not parse is not "gone".
pub fn owner_gone(record: &str, me: u32) -> bool {
    let mut parts = record.split_whitespace();
    let Some(pid) = parts.next().and_then(|p| p.parse::<u32>().ok()) else { return false };
    if pid == me {
        return false;
    }
    if !process_exists(pid) {
        return true;
    }
    match (parts.next().and_then(|s| s.parse::<u64>().ok()), start_time(pid)) {
        (Some(recorded), Some(now)) => recorded != now,
        _ => false,
    }
}

#[cfg(unix)]
pub fn process_exists(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether the process exists; EPERM means it does, and is not ours.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

#[cfg(not(unix))]
pub fn process_exists(_: u32) -> bool {
    true
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn sleeper() -> std::process::Child {
        Command::new("sleep").arg("60").stdout(Stdio::null()).spawn().unwrap()
    }

    #[test]
    fn a_live_process_with_the_recorded_start_is_not_gone() {
        let mut child = sleeper();
        let record = record_for(child.id());
        assert!(!owner_gone(&record, std::process::id()), "{record}");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    /// Pid reuse, made on purpose and not left to the runner: a live process under the recorded pid
    /// but with another start time is a stranger, so the record's owner is gone.
    #[test]
    fn a_reused_pid_is_gone_however_alive_it_looks() {
        let mut child = sleeper();
        let Some(start) = start_time(child.id()) else {
            // No /proc here: nothing to compare, and the pid-only fallback applies.
            child.kill().unwrap();
            child.wait().unwrap();
            return;
        };
        let reused = format!("{} {}", child.id(), start + 1);
        assert!(process_exists(child.id()), "the pid answers signal 0");
        assert!(owner_gone(&reused, std::process::id()), "same pid, other start: another process");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn a_pid_with_no_process_is_gone_and_ours_and_junk_are_not() {
        let mut child = sleeper();
        let record = record_for(child.id());
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(owner_gone(&record, std::process::id()));
        assert!(!owner_gone(&record_for(std::process::id()), std::process::id()));
        assert!(!owner_gone("not a pid", std::process::id()));
    }

    #[test]
    fn an_old_pid_only_record_still_works() {
        // A pid-only record for a live process is kept: with no start time there is nothing to compare.
        let mut live = sleeper();
        assert!(!owner_gone(&live.id().to_string(), std::process::id()));
        live.kill().unwrap();
        live.wait().unwrap();
    }
}
