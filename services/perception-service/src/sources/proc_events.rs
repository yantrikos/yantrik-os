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
use std::path::{Path, PathBuf};

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
                let parent = read_ppid(pid);
                let parent_exe = parent.and_then(read_exe);
                let child_exe = read_exe(pid);
                if is_own_chatter(parent_exe.as_deref(), child_exe.as_deref(), &command) {
                    bus.note_noise();
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

/// The executable a process is running, as a path. A process that has been unlinked since it
/// started reads back with a trailing " (deleted)", which is not part of the path.
fn read_exe(pid: i32) -> Option<PathBuf> {
    let link = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let text = link.to_string_lossy();
    let text = text.strip_suffix(" (deleted)").unwrap_or(&text);
    Some(PathBuf::from(text))
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
    const MARKERS: [&str; 7] =
        ["--password", "--passwd", "--token", "--secret", "--api-key", "--apikey", "--auth"];

    // `-p` gets its own rule, and only for the programs where it means a password.
    //
    // It used to sit in the list above, matched with `starts_with`, which destroyed every long
    // flag beginning with those two characters. Found in the journal after an agent audited the
    // filesystem: `find / -maxdepth 4 -perm -o+w` was recorded as `find '/' -maxdepth 4
    // -p<redacted> -o+w`, and the record of a security audit became unreadable. `-print`,
    // `-path`, `-pipe` and `-pretty` all had the same fate.
    //
    // This is the same shape as an over-broad rule in the companion's audit log — `auth` matching
    // `author` — and the same lesson: over-redaction has a cost too. A record nobody can read
    // stops being consulted, and a record nobody consults is not an audit.
    //
    // So `-p` is a password only for the handful of tools that define it that way. Anywhere else
    // it is an ordinary flag and stays legible.
    const DASH_P_IS_A_PASSWORD: [&str; 4] = ["mysql", "mysqldump", "mysqladmin", "psql"];

    let program = command
        .split(' ')
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let dash_p_hides_a_secret = DASH_P_IS_A_PASSWORD.contains(&program.as_str());

    command
        .split(' ')
        .map(|arg| {
            let lower = arg.to_lowercase();
            for marker in MARKERS {
                // `--password=x` and `--token=abc` hide the value in the same argument.
                if lower.starts_with(marker) && arg.len() > marker.len() {
                    return format!("{marker}<redacted>");
                }
            }
            // `mysql -pHunter2`: the value is jammed against the flag with no separator, so the
            // whole argument goes.
            if dash_p_hides_a_secret && lower.starts_with("-p") && arg.len() > 2 {
                return "-p<redacted>".to_string();
            }
            arg.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where our own binaries live. The executable path is what identifies a program, not the name it
/// chose for itself (`comm`), so a copy of `yos` in `/tmp` is not ours.
const YANTRIK_BIN: &str = "/opt/yantrik/bin";

/// The Wayland compositor the shell runs under. Its helpers are the OS polling itself too.
const LABWC: &str = "/usr/bin/labwc";

/// Our own services, by exact binary name. A parent is only "ours" if it is one of these, never
/// by a `-service` suffix: `comm` is chosen by the program itself, so any program could hide its
/// children by naming itself `x-service`.
const OUR_SERVICES: &[&str] = &[
    "a11y-service",
    "calendar-service",
    "email-service",
    "network-service",
    "notes-service",
    "notifications-service",
    "perception-journal",
    "perception-service",
    "system-monitor-service",
    "weather-service",
];

/// Helper binaries the shell or a service polls. A binary goes here only if the shell or a service
/// polls it, never a user-facing app.
const SHELL_HELPERS: &[&str] = &[
    "nmcli", "wl-paste", "wl-copy", "cat", "pactl", "upower", "ip", "iw", "brightnessctl", "wpctl",
];

/// Whether a launch is the OS's own machinery rather than a person's or an agent's work.
///
/// The shell (`yantrik-ui`, `labwc`) and our own services spawn helpers constantly — `nmcli`,
/// `wl-paste`, `cat` — and the agent reads perception with `yos`. Those are the firehose that
/// fills the ring, not signal. A terminal such as `yantrik-terminal` is different: commands run
/// there are real work, so the parent is matched exactly, never by the `yantrik` prefix.
///
/// The decision is made from the executables (`/proc/<pid>/exe`), not the names the programs chose
/// for themselves: `comm` is truncated to 15 characters and is whatever the program says it is, so
/// a copy of `yos` in `/tmp` or a child of a process that merely calls itself `yantrik-ui` must
/// not be hidden.
fn is_own_chatter(parent_exe: Option<&Path>, child_exe: Option<&Path>, command: &str) -> bool {
    let program = command
        .split(' ')
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default();

    // A perception read is the feedback loop: `yos perception` reads what we just recorded. The
    // executable must be our own `yos`, not any program that happens to be named `yos`.
    if child_exe == Some(Path::new(&format!("{YANTRIK_BIN}/yos")))
        && command.split(' ').nth(1) == Some("perception")
    {
        return true;
    }

    // A helper spawned by the shell or one of our services is the OS polling itself.
    let parent_is_ours = match parent_exe {
        Some(exe) => {
            exe == Path::new(LABWC)
                || (exe.starts_with(YANTRIK_BIN)
                    && exe
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|name| name == "yantrik-ui" || OUR_SERVICES.contains(&name)))
        }
        None => false,
    };
    if parent_is_ours && SHELL_HELPERS.contains(&program) {
        return true;
    }

    false
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
    }

    #[test]
    fn an_ordinary_flag_that_begins_with_dash_p_survives() {
        // Found in the journal, in the record of an agent auditing this machine for
        // world-writable files. The command it actually ran was legible; the record of it was
        // not, because every long flag starting with `-p` was being blanked.
        assert_eq!(
            redact("find / -maxdepth 4 -perm -o+w -not -path /proc/*"),
            "find / -maxdepth 4 -perm -o+w -not -path /proc/*",
            "a security audit must stay readable in the record of it"
        );
        assert_eq!(redact("find . -name x -print"), "find . -name x -print");
        assert_eq!(redact("git log --pretty=oneline"), "git log --pretty=oneline");
        assert_eq!(redact("gcc -pipe -O2 main.c"), "gcc -pipe -O2 main.c");
        assert_eq!(redact("ssh -p 2222 host"), "ssh -p 2222 host");
    }

    #[test]
    fn dash_p_still_hides_a_password_where_it_is_one() {
        // The case the rule exists for, unchanged. Narrowing it to the programs that define `-p`
        // as a password must not stop it working for them.
        assert_eq!(redact("mysql -pHunter2 db"), "mysql -p<redacted> db");
        assert_eq!(redact("/usr/bin/mysqldump -pS3cret schema"), "/usr/bin/mysqldump -p<redacted> schema");
        assert_eq!(redact("psql -pletmein"), "psql -p<redacted>");
        assert_eq!(redact("cargo build --release"), "cargo build --release");
    }

    #[test]
    fn the_os_polling_itself_is_filtered_but_real_work_is_not() {
        let yantrik_ui = Path::new("/opt/yantrik/bin/yantrik-ui");
        let system_monitor = Path::new("/opt/yantrik/bin/system-monitor-service");
        let bash = Path::new("/usr/bin/bash");
        let labwc = Path::new("/usr/bin/labwc");
        let yos = Path::new("/opt/yantrik/bin/yos");

        // The shell and our services polling helpers is the OS talking to itself.
        assert!(is_own_chatter(Some(yantrik_ui), Some(Path::new("/usr/bin/nmcli")), "nmcli -w 2 device"));
        assert!(is_own_chatter(Some(system_monitor), Some(Path::new("/usr/bin/cat")), "cat /proc/stat"));
        assert!(is_own_chatter(Some(bash), Some(yos), "/opt/yantrik/bin/yos perception 20"));

        // Real work is not chatter, even when it looks close.
        assert!(!is_own_chatter(Some(yantrik_ui), Some(Path::new("/usr/bin/firefox")), "firefox"));
        assert!(!is_own_chatter(Some(labwc), Some(Path::new("/usr/bin/foot")), "foot"));
        assert!(!is_own_chatter(Some(bash), Some(yos), "yos act shell open_app name=notes"));
        // A copy of `yos` somewhere else is not ours.
        assert!(!is_own_chatter(Some(bash), Some(Path::new("/tmp/yos")), "/tmp/yos perception 20"));
        // A program that merely calls itself `yantrik-ui` is not the shell.
        assert!(!is_own_chatter(Some(Path::new("/tmp/yantrik-ui")), Some(Path::new("/usr/bin/nmcli")), "nmcli"));
        // A process that exited before we could read its exe is not chatter.
        assert!(!is_own_chatter(None, None, "nmcli"));
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
