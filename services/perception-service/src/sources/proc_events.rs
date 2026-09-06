//! What is running, told by the kernel rather than counted by us.
//!
//! `crates/yantrik-os/src/processes.rs` sleeps for two seconds and calls
//! `refresh_processes(All)` — a full walk of `/proc` on a timer. It is expensive in proportion to
//! how many processes exist rather than how much is happening, and it is blind between ticks: a
//! compile, a script, an installer that runs and exits inside one interval never existed.
//!
//! The netlink process connector is the opposite shape. The kernel pushes an event at the moment
//! of `execve` and `exit`, costs nothing while nothing runs, and misses nothing however short.
//! It has been in the kernel since 2.6.15; it just needs `CAP_NET_ADMIN` to join the multicast
//! group, which is most of the reason this is a separate privileged service.
//!
//! Everything here parses the wire bytes by offset rather than casting to a struct. The kernel
//! places `proc_event` at an offset that is only four-byte aligned, so a `#[repr(C)]` cast would
//! be reading a `u64` from a misaligned address — undefined behaviour that happens to work on
//! x86 and does not elsewhere.

use std::collections::HashMap;
use std::io::Read;

use crate::bus::Bus;
use crate::observation::{Actor, Kind};

const NETLINK_CONNECTOR: libc::c_int = 11;
const CN_IDX_PROC: u32 = 1;
const CN_VAL_PROC: u32 = 1;
const PROC_CN_MCAST_LISTEN: u32 = 1;

const PROC_EVENT_EXEC: u32 = 0x0000_0002;
const PROC_EVENT_EXIT: u32 = 0x8000_0000;

/// Offsets into a received netlink datagram.
const CN_MSG: usize = 16; // after nlmsghdr
const PROC_EVENT: usize = CN_MSG + 20; // after cn_msg
const EVENT_DATA: usize = PROC_EVENT + 16; // after what, cpu, timestamp_ns

/// How many live processes we remember by name.
///
/// Only so an exit can say *what* exited. Bounded because a machine that forks in a loop must not
/// be able to grow this: past the limit the oldest entries go and those exits are simply reported
/// without a name, which is a better failure than running out of memory.
const REMEMBERED: usize = 4096;

/// Whether the pids the connector reports mean anything here.
///
/// The connector reports `task->pid`: the pid in the *initial* namespace. A service running inside
/// a PID namespace — a container, or any WSL distro — sees entirely different numbers in `/proc`,
/// so every lookup misses and every observation comes back nameless.
///
/// It took a hex dump of the wire bytes to find. The offsets were right all along and the numbers
/// were simply from somewhere else: an exec reported pid 12910 on a machine whose own pids were
/// past 554000. Nothing in the event says so; it just quietly attributes nothing.
///
/// `NSpid` in `/proc/self/status` is the obvious check and it does not work: read from inside a
/// namespace it lists one entry, because the outer pids are precisely what cannot be seen. The
/// namespace's inode can be compared instead — the initial one is a fixed kernel constant.
///
/// There is no fix from inside. The honest response is to say so, once, in the stream where
/// whatever is reading will notice.
fn attribution_possible() -> Result<(), String> {
    /// `PROC_PID_INIT_INO` — the inode of the initial PID namespace, fixed in the kernel.
    const INITIAL: &str = "pid:[4026531836]";

    let ours = std::fs::read_link("/proc/self/ns/pid")
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    if ours.is_empty() {
        // No `/proc/self/ns` at all. Unusual, and not something to guess about.
        return Err("cannot read /proc/self/ns/pid, so pid attribution is unverifiable".into());
    }
    if ours != INITIAL {
        return Err(format!(
            "this process is in PID namespace {ours}, not the initial one; the connector reports \
             initial-namespace pids which cannot be resolved from here, so launches would be \
             unattributable. File events are unaffected."
        ));
    }
    Ok(())
}

/// What a wait status actually means.
///
/// `exit_code` in the connector's exit event is a raw wait status, not an exit code, and
/// `exit_signal` is the signal the dying process sends its *parent* — almost always `SIGCHLD`.
/// Reading them as "exit code" and "killed by" turned every ordinary process ending into
/// "killed by signal 17", which was both wrong and alarming.
fn decode_status(status: u32) -> (i32, i32) {
    let terminating = (status & 0x7f) as i32;
    if terminating == 0 {
        (((status >> 8) & 0xff) as i32, 0)
    } else {
        (0, terminating)
    }
}

/// Read events from an already-subscribed socket.
///
/// The socket is opened on the main thread while the process still holds `CAP_NET_ADMIN`, and
/// handed here afterwards. By the time this runs the capability is gone — which is the point: the
/// privilege was needed to *start* watching, not to keep watching.
pub fn run(bus: Bus, sock: libc::c_int) {
    tracing::info!("Process connector listening");

    // SAFETY: `sock` was created by `subscribe` and ownership moves here; nothing else closes it.
    let mut file = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(sock) };
    if let Err(why) = attribution_possible() {
        bus.push(Kind::SourceFailed { source: "processes".into(), reason: why.clone() }, None);
        tracing::warn!(reason = %why, "Process events cannot be attributed here");
        return;
    }

    let mut buf = [0u8; 4096];
    let mut live: HashMap<i32, String> = HashMap::new();

    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                bus.push(
                    Kind::SourceFailed { source: "processes".into(), reason: e.to_string() },
                    None,
                );
                tracing::warn!(error = %e, "Process connector stopped");
                return;
            }
        };
        if n < EVENT_DATA + 8 {
            continue;
        }

        match u32::from_ne_bytes(buf[PROC_EVENT..PROC_EVENT + 4].try_into().unwrap()) {
            PROC_EVENT_EXEC => {
                let pid = i32::from_ne_bytes(buf[EVENT_DATA..EVENT_DATA + 4].try_into().unwrap());
                // Read after the fact, so a process that has already gone leaves nothing rather
                // than a guess. This is the one race inherent to watching from outside, and it is
                // why a launch nobody could name is dropped instead of reported as "something".
                let name = read_proc(pid, "comm").unwrap_or_default();
                let command = read_cmdline(pid).unwrap_or_else(|| name.clone());
                if command.is_empty() {
                    continue;
                }
                if live.len() >= REMEMBERED {
                    live.clear();
                }
                live.insert(pid, name.clone());
                bus.push(
                    Kind::Launched { command },
                    Some(Actor { pid, name, parent: read_ppid(pid) }),
                );
            }
            PROC_EVENT_EXIT => {
                if n < EVENT_DATA + 24 {
                    continue;
                }
                let pid = i32::from_ne_bytes(buf[EVENT_DATA..EVENT_DATA + 4].try_into().unwrap());
                let status =
                    u32::from_ne_bytes(buf[EVENT_DATA + 8..EVENT_DATA + 12].try_into().unwrap());
                let (code, signal) = decode_status(status);

                let name = live.remove(&pid);
                // Every fork's exit arrives here, and almost all of them are a shell reaping a
                // pipeline. Report one only if we said it started, or if it ended badly — a
                // failure is worth knowing about even from something we never announced.
                let interesting = name.is_some() || code != 0 || signal != 0;
                if !interesting {
                    continue;
                }
                bus.push(
                    Kind::Ended { exit_code: code, signal },
                    name.map(|name| Actor { pid, name, parent: None }),
                );
            }
            _ => {}
        }
    }
}

/// Join the process multicast group and ask to be sent events.
///
/// Needs `CAP_NET_ADMIN`, so it is called once on the main thread before the capability is
/// dropped. Everything after this point is reading from a descriptor.
pub fn subscribe() -> Result<libc::c_int, String> {
    // SAFETY: a plain socket() call with constant arguments.
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM, NETLINK_CONNECTOR) };
    if fd < 0 {
        return Err(format!("socket: {}", std::io::Error::last_os_error()));
    }

    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as u16;
    addr.nl_groups = CN_IDX_PROC;
    addr.nl_pid = 0; // let the kernel assign, so two instances do not collide

    // SAFETY: `addr` is a correctly sized sockaddr_nl and `fd` is open.
    let rc = unsafe {
        libc::bind(
            fd,
            &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        let err = std::io::Error::last_os_error();
        // SAFETY: closing our own fd on the failure path.
        unsafe { libc::close(fd) };
        return Err(match err.raw_os_error() {
            // The common one, and worth naming precisely: joining this group is privileged.
            Some(libc::EPERM) => "bind: needs CAP_NET_ADMIN".to_string(),
            _ => format!("bind: {err}"),
        });
    }

    // nlmsghdr (16) + cn_msg (20) + the op (4).
    let mut msg = [0u8; 40];
    msg[0..4].copy_from_slice(&40u32.to_ne_bytes()); // nlmsg_len
    msg[4..6].copy_from_slice(&3u16.to_ne_bytes()); // NLMSG_DONE
    // flags, seq and pid stay zero: not a request, no sequence to match, kernel-assigned port.

    msg[CN_MSG..CN_MSG + 4].copy_from_slice(&CN_IDX_PROC.to_ne_bytes());
    msg[CN_MSG + 4..CN_MSG + 8].copy_from_slice(&CN_VAL_PROC.to_ne_bytes());
    msg[CN_MSG + 16..CN_MSG + 18].copy_from_slice(&4u16.to_ne_bytes()); // payload length
    msg[CN_MSG + 20..CN_MSG + 24].copy_from_slice(&PROC_CN_MCAST_LISTEN.to_ne_bytes());

    // SAFETY: `msg` is a live buffer of exactly the length passed.
    let sent = unsafe { libc::send(fd, msg.as_ptr() as *const libc::c_void, msg.len(), 0) };
    if sent < 0 {
        let err = std::io::Error::last_os_error();
        // SAFETY: closing our own fd on the failure path.
        unsafe { libc::close(fd) };
        return Err(format!("send: {err}"));
    }
    Ok(fd)
}

fn read_proc(pid: i32, file: &str) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/{file}"))
        .ok()
        .map(|s| s.trim().to_string())
}

/// The command line, NUL-separated in `/proc`, as something readable.
fn read_cmdline(pid: i32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let text: String = raw
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).to_string())
        .collect::<Vec<_>>()
        .join(" ");
    // Long enough to identify a command, short enough that a build with two hundred flags does
    // not fill the ring by itself.
    Some(redact(&text).chars().take(300).collect())
}

/// Blank out arguments that carry a secret.
///
/// `/proc/<pid>/cmdline` is world-readable and people put passwords and tokens on command lines
/// anyway. Anything this service records may end up in the companion's memory, which is durable
/// and searchable, so a secret that passes through here does not pass through once.
fn redact(command: &str) -> String {
    const MARKERS: [&str; 8] =
        ["--password", "--passwd", "--token", "--secret", "--api-key", "--apikey", "--auth", "-p"];
    command
        .split(' ')
        .map(|arg| {
            let lower = arg.to_lowercase();
            for marker in MARKERS {
                // `--password=x` and `-pSECRET` both hide the value in the same argument.
                if lower.starts_with(marker) && arg.len() > marker.len() {
                    return format!("{marker}<redacted>");
                }
            }
            arg.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn read_ppid(pid: i32) -> Option<i32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("PPid:"))
        .and_then(|v| v.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The offsets are the whole correctness of this file, and they are the thing most likely to
    /// be got wrong by someone reading `cn_proc.h` quickly.
    #[test]
    fn the_wire_offsets_match_the_kernels_layout() {
        assert_eq!(CN_MSG, 16, "nlmsghdr is 16 bytes");
        assert_eq!(PROC_EVENT, 36, "cn_msg is 20 bytes and follows the nlmsghdr");
        assert_eq!(EVENT_DATA, 52, "proc_event is what(4) + cpu(4) + timestamp_ns(8) then a union");
    }

    #[test]
    fn a_subscribe_message_is_exactly_what_the_connector_expects() {
        // Rebuilt here rather than exposed from `subscribe`, so the test would still catch a
        // change to the layout above even if the socket call were rewritten.
        let mut msg = [0u8; 40];
        msg[0..4].copy_from_slice(&40u32.to_ne_bytes());
        msg[4..6].copy_from_slice(&3u16.to_ne_bytes());
        msg[CN_MSG..CN_MSG + 4].copy_from_slice(&CN_IDX_PROC.to_ne_bytes());
        msg[CN_MSG + 20..CN_MSG + 24].copy_from_slice(&PROC_CN_MCAST_LISTEN.to_ne_bytes());

        assert_eq!(u32::from_ne_bytes(msg[0..4].try_into().unwrap()), 40);
        assert_eq!(u16::from_ne_bytes(msg[4..6].try_into().unwrap()), 3);
        assert_eq!(u32::from_ne_bytes(msg[36..40].try_into().unwrap()), PROC_CN_MCAST_LISTEN);
    }

    #[test]
    fn a_wait_status_is_not_an_exit_code() {
        // The field is a wait status: the low seven bits are the terminating signal and the next
        // eight are the exit code. Reading it as an exit code, and `exit_signal` as "killed by",
        // reported every ordinary process ending as "killed by signal 17".
        assert_eq!(decode_status(0x0000), (0, 0), "a clean exit");
        assert_eq!(decode_status(0x0100), (1, 0), "exit(1)");
        assert_eq!(decode_status(0x7f00), (127, 0), "command not found");
        assert_eq!(decode_status(9), (0, 9), "SIGKILL");
        assert_eq!(decode_status(11), (0, 11), "a segfault");
    }

    #[test]
    fn attribution_is_checked_against_the_namespace_we_are_actually_in() {
        // Whichever way this machine is set up, the answer must match `/proc/self/ns/pid` rather
        // than a guess — and where it says no, it must say why in terms someone can act on.
        let ours = std::fs::read_link("/proc/self/ns/pid")
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let verdict = attribution_possible();
        if ours == "pid:[4026531836]" {
            assert!(verdict.is_ok(), "the initial namespace must allow attribution");
        } else {
            let why = verdict.expect_err("a nested namespace cannot attribute connector pids");
            assert!(why.contains("namespace"), "the reason must name the cause: {why}");
            assert!(
                why.contains("File events are unaffected"),
                "and must not imply the whole service is blind: {why}"
            );
        }
    }

    #[test]
    fn secrets_on_a_command_line_do_not_reach_the_record() {
        // Anything recorded here can end up in durable, searchable memory. A password that passes
        // through does not pass through once.
        assert_eq!(redact("mysql -pHunter2 db"), "mysql -p<redacted> db");
        assert_eq!(redact("curl --token=abc123 host"), "curl --token<redacted> host");
        // A bare flag carries nothing and should survive intact.
        assert_eq!(redact("git commit -p"), "git commit -p");
        assert_eq!(redact("cargo build --release"), "cargo build --release");
    }

    #[test]
    fn a_command_line_is_readable_and_bounded() {
        // Our own, which is guaranteed to exist while the test runs.
        let me = std::process::id() as i32;
        let cmd = read_cmdline(me).expect("we can read our own cmdline");
        assert!(!cmd.is_empty());
        assert!(!cmd.contains('\0'), "NUL separators must be rendered as spaces");
        assert!(cmd.chars().count() <= 300);
    }
}
