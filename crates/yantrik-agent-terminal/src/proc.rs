//! What `/proc` says about a job's processes: which of them are in the foreground, and whether one
//! of those is sitting in `read(2)` on the terminal.
//!
//! Every read here is allowed to fail — a process can exit between two files — and failing means
//! "not waiting", never an error: this is a hint to show a person, not a fact to act on.

use std::os::fd::RawFd;
use std::path::Path;

/// How many processes of one job are looked at. A job with more than this in its tree is a build,
/// not a prompt.
const FAMILY_BOUND: usize = 64;

/// `read`'s number, where this is built for an architecture we know it on. Elsewhere the check
/// falls back to `wchan`.
#[cfg(target_arch = "x86_64")]
const READ_NR: Option<&str> = Some("0");
#[cfg(target_arch = "aarch64")]
const READ_NR: Option<&str> = Some("63");
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const READ_NR: Option<&str> = None;

/// The fields of `/proc/<pid>/stat` this module needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stat {
    pub state: char,
    pub pgrp: i32,
    pub session: i32,
}

/// Parse state (field 3), pgrp (field 5) and session (field 6), from the last `)` — the command name in
/// field 2 is not escaped, so it can hold spaces and parentheses. The same rule as `peer_identity::parse_stat`.
pub(crate) fn parse_stat(text: &str) -> Option<Stat> {
    let tail = &text[text.rfind(')')? + 1..];
    let fields: Vec<&str> = tail.split_whitespace().collect();
    Some(Stat {
        state: fields.first()?.chars().next()?,
        pgrp: fields.get(5 - 3)?.parse().ok()?,
        session: fields.get(6 - 3)?.parse().ok()?,
    })
}

fn stat(pid: i32) -> Option<Stat> {
    parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// The leader and everything under it, breadth first, bounded.
///
/// From `/proc/<pid>/task/<tid>/children` rather than a scan of all of `/proc`: it touches only
/// this job's tree, which is what a check run once a second per silent job should cost.
fn family(leader: i32) -> Vec<i32> {
    let mut out = vec![leader];
    let mut at = 0;
    while at < out.len() && out.len() < FAMILY_BOUND {
        let pid = out[at];
        at += 1;
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else { continue };
        for task in tasks.flatten() {
            let Ok(children) = std::fs::read_to_string(task.path().join("children")) else {
                continue;
            };
            for child in children.split_whitespace().filter_map(|c| c.parse::<i32>().ok()) {
                if !out.contains(&child) && out.len() < FAMILY_BOUND {
                    out.push(child);
                }
            }
        }
    }
    out
}

/// Whether `pid` is blocked reading `tty` (or `/dev/tty`, which is where `sudo` and `ssh` ask).
fn reads_terminal(pid: i32, tty: &Path) -> bool {
    let is_terminal = |fd: &str| {
        std::fs::read_link(format!("/proc/{pid}/fd/{fd}"))
            .is_ok_and(|target| target == tty || target == Path::new("/dev/tty"))
    };
    // `/proc/<pid>/syscall` is "<nr> <arg0> …" while blocked, arg0 in hex: for read, the fd.
    // Readable by the process's parent chain (ptrace_scope 1 allows descendants), which is us.
    if let (Some(read_nr), Ok(text)) = (READ_NR, std::fs::read_to_string(format!("/proc/{pid}/syscall"))) {
        let mut fields = text.split_whitespace();
        if let (Some(nr), Some(fd)) = (fields.next(), fields.next()) {
            if nr != read_nr {
                return false;
            }
            let fd = i64::from_str_radix(fd.trim_start_matches("0x"), 16).unwrap_or(-1);
            return fd >= 0 && is_terminal(&fd.to_string());
        }
    }
    // Without `syscall`, the wait channel: the tty line discipline's own read is unambiguous, and
    // the generic `wait_woken` it sleeps in on newer kernels counts only if stdin is the terminal.
    match std::fs::read_to_string(format!("/proc/{pid}/wchan")).as_deref().map(str::trim) {
        Ok("n_tty_read") | Ok("tty_read") => true,
        Ok("wait_woken") => is_terminal("0"),
        _ => false,
    }
}

/// Whether a process in the terminal's foreground group is asleep reading it.
///
/// `master` is the PTY master: `tcgetpgrp` on it answers for the slave on Linux.
pub(crate) fn waiting_on_terminal(master: RawFd, leader: i32, tty: &Path) -> bool {
    let foreground = unsafe { libc::tcgetpgrp(master) };
    if foreground <= 0 || tty.as_os_str().is_empty() {
        return false;
    }
    family(leader).into_iter().any(|pid| {
        stat(pid).is_some_and(|s| s.pgrp == foreground && s.state == 'S') && reads_terminal(pid, tty)
    })
}

/// Every process in session `sid` that is not a `bwrap`, from a scan of `/proc`: the command and
/// what it started, as a host sees them from outside the pid namespace they are in. The `bwrap`s
/// are left out because they are the namespace's way of ending: they die of a TERM, and the
/// kernel then kills everything the namespace holds.
pub(crate) fn session_members(sid: i32) -> Vec<i32> {
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        let in_session = stat(pid).is_some_and(|s| s.session == sid);
        let is_bwrap = std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim() == "bwrap");
        if in_session && !is_bwrap {
            out.push(pid);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_is_read_from_the_last_parenthesis() {
        let text = "4242 (my (odd) name) S 1 4240 4240 34816 4240 4194560 0 0 0 0 0 0 0 0 20 0 1 0 1234 0 0";
        assert_eq!(parse_stat(text), Some(Stat { state: 'S', pgrp: 4240, session: 4240 }));
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn a_session_is_listed_by_the_processes_in_it() {
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let sid = unsafe { libc::getsid(0) };
        let members = session_members(sid);
        let _ = child.kill();
        let _ = child.wait();
        assert!(members.contains(&(std::process::id() as i32)) && members.contains(&(child.id() as i32)), "{members:?}");
        assert!(session_members(i32::MAX).is_empty());
    }
}
