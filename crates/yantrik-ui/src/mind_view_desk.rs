//! The Mind desk's process, started and known by the shell (#1023).
//!
//! The desk (`apps/mind-desk`) is a window of Mind View, where a Mind acting can read and press
//! things. What the shell answers it must therefore be answered to the desk and to nothing else, and
//! nothing about a process can say it is the desk but the shell's own record of having started it:
//!
//! - **The shell starts it**, as a child of its own, on the nested display with the person's
//!   session variables cleared (`Seat::display`, as for every app a Mind opens). It is not started
//!   by the nested compositor's startup script: a process the script starts has a script and a
//!   compositor in its ancestry, whose words (`mind`, `view`, `seat`) the shell's by-name guess at
//!   which Mind is calling matched with the Yantrik Mind, so the desk was refused as a Mind.
//! - **It records the desk's pid and its start time** (`/proc/<pid>/stat`), and a peer is the desk
//!   only if the pid the kernel names on the socket and the start time both match
//!   ([`is_the_desk`]). The start time is what a reused pid cannot have. Nothing is exempted by the
//!   path of an executable: any Mind can run that binary, and the one it runs is not this one.
//! - **It restarts it** when it ends, with a pause that grows, for as long as Mind View is there.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::mind_view::Seat;

/// The desk's program, as installed beside the shell.
pub const PROGRAM: &str = "yantrik-mind-desk";

/// The pause before the desk is started again, doubling to the longest, and the run that is
/// long enough to start the count over.
const PAUSE_FROM: Duration = Duration::from_secs(1);
const PAUSE_UP_TO: Duration = Duration::from_secs(30);
const RAN_LONG_ENOUGH: Duration = Duration::from_secs(60);

/// The desk this shell started: which process, told apart from any other that gets its pid later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub pid: u32,
    /// Clock ticks after boot at which it started (`/proc/<pid>/stat`, field 22).
    pub started: u64,
}

static DESK: Mutex<Option<Record>> = Mutex::new(None);

fn record() -> Option<Record> {
    *DESK.lock().unwrap_or_else(|p| p.into_inner())
}

fn set_record(to: Option<Record>) {
    *DESK.lock().unwrap_or_else(|p| p.into_inner()) = to;
}

/// Forget the desk recorded, but only if it is `pid`: a run that ends must not wipe the record of
/// a desk another run has started since.
fn clear_record_of(pid: u32) {
    let mut held = DESK.lock().unwrap_or_else(|p| p.into_inner());
    if held.is_some_and(|d| d.pid == pid) {
        *held = None;
    }
}

/// Which supervisor is the current one. Mind View restarting on the display name it had (labwc
/// takes the first free `wayland-N`) must not leave the old supervisor running beside the new: two
/// desks on one display, both placed by the same rules, and each ending wiping the other's record.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn is_current(generation: u64) -> bool {
    GENERATION.load(Ordering::SeqCst) == generation
}

/// The desk's program beside the shell, or in the install directory, and nowhere else. Not on
/// `PATH`: a runs-as-you Mind that can write a directory on it must not get its own binary started
/// and blessed as the desk.
fn find_desk() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    [beside, Some(PathBuf::from("/opt/yantrik/bin"))]
        .into_iter()
        .flatten()
        .map(|dir| dir.join(PROGRAM))
        .find(|p| p.is_file())
}

/// The start time a `/proc/<pid>/stat` line holds: field 22, counted after the command name, which
/// is in brackets and may itself hold spaces and brackets.
pub fn start_time_in(stat: &str) -> Option<u64> {
    let after_name = stat.rsplit_once(')')?.1;
    // Fields from the third (the state) on; the start time is the twenty-second overall.
    after_name.split_whitespace().nth(19)?.parse().ok()
}

fn start_time(pid: u32) -> Option<u64> {
    start_time_in(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// Whether the peer at `pid` is the desk `record` names, with the start time of a pid asked for by
/// `start_of`. No record, no peer, another pid, or the same pid started at another time: no.
pub fn is_the_desk(record: Option<Record>, pid: Option<u32>, start_of: impl Fn(u32) -> Option<u64>) -> bool {
    match (record, pid) {
        (Some(desk), Some(pid)) => pid == desk.pid && start_of(pid) == Some(desk.started),
        _ => false,
    }
}

/// `Ok` when the call being dispatched on this thread comes from the desk this shell started.
/// Read where the caller is in scope: inside the handler, on the dispatch's thread.
pub fn require_the_desk() -> Result<(), String> {
    let pid = yantrik_app_runtime::control::caller().and_then(|c| u32::try_from(c.pid).ok()).filter(|p| *p > 0);
    if is_the_desk(record(), pid, start_time) {
        Ok(())
    } else {
        Err(REFUSED.to_string())
    }
}

/// What a caller that is not the desk is told.
pub const REFUSED: &str = "this is answered only to the Mind desk the shell started in Mind View. `read_agent` reads \
    your own session, and `describe shell` says what you may know of the others.";

/// Start the desk on `seat` and keep it running for as long as that is Mind View's display.
pub fn supervise(seat: Seat) {
    let Some(program) = find_desk() else {
        tracing::info!("the Mind desk is not installed beside the shell, so Mind View has no bars");
        return;
    };
    // This is the supervisor now; any earlier one stops at its next look.
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let spawned = std::thread::Builder::new().name("mind-desk".into()).spawn(move || {
        let mut pause = PAUSE_FROM;
        while is_current(generation) && crate::mind_view::display_now().as_deref() == Some(seat.wayland.as_str()) {
            let began = Instant::now();
            run_once(&program, &seat);
            if began.elapsed() >= RAN_LONG_ENOUGH {
                pause = PAUSE_FROM;
            }
            std::thread::sleep(pause);
            pause = (pause * 2).min(PAUSE_UP_TO);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the thread that keeps the Mind desk running");
    }
}

/// One run of the desk, from its start to its end.
fn run_once(program: &Path, seat: &Seat) {
    let mut command = std::process::Command::new(program);
    // On the nested display, with everything that names the person's session cleared, and marked
    // as drawn for a Mind: the same environment as every app a Mind opens.
    seat.display().apply(&mut command);
    command
        .env_remove("SLINT_FULLSCREEN")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            tracing::warn!(error = %e, "could not start the Mind desk");
            return;
        }
    };
    // Known only with a start time to tell it by: without one, nothing is answered to it.
    let pid = child.id();
    set_record(start_time(pid).map(|started| Record { pid, started }));
    let ended = child.wait();
    clear_record_of(pid);
    tracing::info!(?ended, "the Mind desk ended");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `/proc/<pid>/stat` line, with a command name that holds spaces and a bracket.
    const STAT: &str = "4242 (yantrik (mind) desk) S 1 4242 4242 0 -1 4194560 12345 0 0 0 10 5 0 0 20 0 7 0 987654 1234567 8910 \
        18446744073709551615 1 1 0 0 0 0 0 4096 0 0 0 0 17 3 0 0 0 0 0";

    #[test]
    fn the_start_time_is_field_twenty_two_whatever_the_command_is_called() {
        assert_eq!(start_time_in(STAT), Some(987654));
        assert_eq!(start_time_in("1 (a) S 1"), None);
        assert_eq!(start_time_in("no brackets"), None);
    }

    #[test]
    fn only_the_process_the_shell_started_is_the_desk() {
        let desk = Some(Record { pid: 700, started: 5000 });
        let starts = |pid: u32| match pid {
            700 => Some(5000),
            701 => Some(5000),
            _ => None,
        };
        assert!(is_the_desk(desk, Some(700), starts));
        // Another process, even one that started at the same instant.
        assert!(!is_the_desk(desk, Some(701), starts));
        // The same pid, reused by a process that started later.
        assert!(!is_the_desk(desk, Some(700), |_| Some(6000)));
        // A pid that has gone, a caller with no pid (no socket), and a shell that started no desk.
        assert!(!is_the_desk(desk, Some(700), |_| None));
        assert!(!is_the_desk(desk, None, starts));
        assert!(!is_the_desk(None, Some(700), starts));
    }

    /// With no desk started in a test, and no socket, nobody is the desk: the refusal is the
    /// default, not a thing that has to be remembered.
    #[test]
    fn a_caller_is_refused_unless_it_is_the_desk() {
        assert_eq!(require_the_desk().unwrap_err(), REFUSED);
    }

    /// Two supervisors on one display (Mind View restarting on the name it had) leave one: the
    /// newest. And a run that ends clears its own desk's record and no other.
    #[test]
    fn a_restart_leaves_one_supervisor_and_a_run_clears_only_its_own_record() {
        let first = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(is_current(first));
        let second = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(!is_current(first), "the old one stops at its next look");
        assert!(is_current(second));

        set_record(Some(Record { pid: 900, started: 1 }));
        clear_record_of(901);
        assert_eq!(record(), Some(Record { pid: 900, started: 1 }), "another run's end does not wipe it");
        clear_record_of(900);
        assert_eq!(record(), None);
    }

    /// The desk is looked for beside the shell and in the install directory, never on `PATH`.
    #[test]
    fn the_desk_is_not_looked_for_on_the_path() {
        let source = include_str!("mind_view_desk.rs").split("#[cfg(test)]").next().unwrap().to_string();
        assert!(!source.contains("env::var_os(\"PATH\")") && !source.contains("find_program"));
    }

    /// Nothing exempts by the program a process runs: the check has no path in it.
    #[test]
    fn the_check_reads_no_executable_path() {
        let source = include_str!("mind_view_desk.rs").split("#[cfg(test)]").next().unwrap().to_string();
        assert!(!source.contains("/proc/{pid}/exe") && !source.contains("read_link"), "the desk is told by pid and start time");
    }
}
