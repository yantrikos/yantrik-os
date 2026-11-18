//! What a kill actually did, and where a reading came from.
//!
//! Both of the things this app was least honest about are classifications, and a classification
//! can be decided without a window. `kill_process` answered `{"killed": pid}` whether the
//! service call, the local fallback, or neither had managed it; and the readings on screen
//! changed from the service's to this process's own the moment the service stopped answering,
//! with nothing anywhere saying so. The rules for both are here, where a test can reach them.

use std::time::{Duration, Instant};

/// Which signal was asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Signal {
    /// Ask the process to stop, and let it run whatever it does on the way out.
    Term,
    /// Take it out of the scheduler. Nothing in userspace can decline this one.
    Kill,
}

impl Signal {
    pub fn as_str(self) -> &'static str {
        match self {
            Signal::Term => "SIGTERM",
            Signal::Kill => "SIGKILL",
        }
    }
}

/// Who did the work — the service that owns this domain, or this process itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Service,
    Local,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Service => "service",
            Source::Local => "local",
        }
    }
}

/// Whether the process table still holds a pid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Liveness {
    Running,
    Gone,
}

// ── Is it still there ───────────────────────────────────────────────

/// How long a kill waits for the process to actually leave the table.
///
/// This runs in a job, and `kill_process` answers when it ends: its caller waits three seconds
/// for that answer (`intents.rs`, `KILL_WITHIN`, the dispatch's old `UI_ROUNDTRIP`). A second is
/// long enough for a SIGTERM handler to finish — they are normally done in single-digit
/// milliseconds — and short enough that a process which is refusing to go is reported as such
/// rather than held on to.
pub const VERIFY_BUDGET: Duration = Duration::from_millis(1000);

/// How often to look while waiting. Twenty milliseconds is below what anyone notices and well
/// above the cost of one `/proc` read.
pub const VERIFY_STEP: Duration = Duration::from_millis(20);

/// Read a process's state out of a line of `/proc/<pid>/stat`.
///
/// The state is the third field, but the second is the executable's own name in parentheses and
/// may contain both spaces and parentheses — `1234 (my (odd) name) S 1 …` is a legal line — so
/// it is found from the last `)` rather than by splitting the line on whitespace.
///
/// `Z` and `X` count as gone. A child that has been killed but not yet waited on by its parent
/// is a zombie: it has exited, its entry is still in the table, and calling that "still
/// running" would make a successful kill report itself as a failure. Any test that spawns its
/// own child and kills it sees exactly this, and so does anything this app kills that is a
/// child of a shell nobody is sitting at.
pub fn liveness_from_stat(stat: &str) -> Liveness {
    let Some(after_comm) = stat.rfind(')').map(|i| &stat[i + 1..]) else {
        // Not a line from /proc/<pid>/stat at all. Nothing is claimed about a pid whose entry
        // cannot be read.
        return Liveness::Running;
    };
    match after_comm.split_whitespace().next() {
        Some("Z") | Some("X") | Some("x") => Liveness::Gone,
        Some(_) => Liveness::Running,
        None => Liveness::Running,
    }
}

/// Whether a pid is a process that is still doing something.
#[cfg(target_os = "linux")]
pub fn liveness(pid: u32) -> Liveness {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => liveness_from_stat(&stat),
        // No entry: the pid is not in the table. This is also how a pid that never existed
        // reads, which is why the kill path checks liveness before it signals anything.
        Err(_) => Liveness::Gone,
    }
}

/// Whether a pid is a process that is still doing something.
///
/// No `/proc` here, so `sysinfo`'s table is the only reading available. It does not distinguish
/// a zombie from a running process, so this path can report an exited child as still running;
/// the service and every machine this app ships to are Linux.
#[cfg(not(target_os = "linux"))]
pub fn liveness(pid: u32) -> Liveness {
    let sys = sysinfo::System::new_all();
    match sys.process(sysinfo::Pid::from_u32(pid)) {
        Some(_) => Liveness::Running,
        None => Liveness::Gone,
    }
}

/// Wait, briefly, for a signalled pid to leave the process table, and say how long it took.
///
/// A kill is a request, not an outcome. `kill(2)` returning 0 says the signal was delivered and
/// nothing at all about whether the process acted on it — a SIGTERM handler may take a moment,
/// and a process is entitled to ignore it. So what this app reports is read from the table
/// afterwards rather than from the syscall's return value.
pub fn wait_until_gone(
    pid: u32,
    budget: Duration,
    step: Duration,
    mut probe: impl FnMut(u32) -> Liveness,
) -> (bool, u64) {
    let started = Instant::now();
    loop {
        if probe(pid) == Liveness::Gone {
            return (true, started.elapsed().as_millis() as u64);
        }
        if started.elapsed() >= budget {
            return (false, started.elapsed().as_millis() as u64);
        }
        std::thread::sleep(step);
    }
}

// ── What one kill did ───────────────────────────────────────────────

/// What a kill attempt was observed to do, as opposed to what it was asked to do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Observed {
    pub pid: u32,
    pub name: String,
    pub signal: Signal,
    pub via: Source,
    pub waited_ms: u64,
}

impl Observed {
    /// The action's answer. Every field here is something that was looked at: the signal that
    /// was sent, the path that sent it, and that the pid was afterwards checked to be gone.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "pid": self.pid,
            "name": self.name,
            "signal": self.signal.as_str(),
            "via": self.via.as_str(),
            "exited": true,
            "confirmed_gone_after_ms": self.waited_ms,
        })
    }

    /// The same thing in the words the person at the window gets.
    pub fn sentence(&self) -> String {
        format!(
            "Ended {} ({}) with {} via the {} path",
            self.name,
            self.pid,
            self.signal.as_str(),
            self.via.as_str()
        )
    }
}

/// The pids this app will not end.
///
/// pid 1 is init and pid 0 is the caller's own process group, which would take the window with
/// it. Anything below that is not a pid at all.
pub fn checked_pid(pid: i32) -> Result<u32, String> {
    if pid <= 1 {
        return Err(format!("{pid} is not a process this app should end"));
    }
    Ok(pid as u32)
}

/// Turn what was observed into the one answer both the button and the action get.
///
/// A process that is still there is not a process that was killed, whatever the syscall said.
/// The two ways that happens read very differently to whoever asked, so they say different
/// things: a declined SIGTERM has an obvious next step, and a SIGKILL that did not land means
/// the process is in a state the kernel will not interrupt.
pub fn classify(
    pid: u32,
    name: &str,
    signal: Signal,
    via: Source,
    exited: bool,
    waited_ms: u64,
) -> Result<Observed, String> {
    if !exited {
        return Err(match signal {
            Signal::Term => format!(
                "{pid} ({name}) was sent SIGTERM and was still running {waited_ms}ms later; \
                 it is declining to exit — force the kill to send SIGKILL"
            ),
            Signal::Kill => format!(
                "{pid} ({name}) was sent SIGKILL and is still in the process table \
                 {waited_ms}ms later; it is most likely stuck in an uninterruptible wait"
            ),
        });
    }
    Ok(Observed { pid, name: to_owned(name), signal, via, waited_ms })
}

fn to_owned(name: &str) -> String {
    if name.trim().is_empty() {
        // The name is read off the list on screen, and the list is the top hundred processes.
        // A pid that is not on it is still a pid worth ending; it just has no name here, and
        // saying "unknown" is truer than printing an empty pair of brackets.
        "unknown".to_string()
    } else {
        name.to_string()
    }
}

/// Why a signal could not be delivered, in the words of the errno the kernel set.
///
/// The reason lives nowhere else. `sysinfo`'s `Process::kill` answers with a bool and
/// `kill_with` with an `Option<bool>`, so the local fallback could not tell "there is no such
/// process" from "that one is not yours" even when it noticed a failure — and it dropped the
/// return value, so it never noticed one.
#[cfg(unix)]
pub fn signal_failure(pid: u32, err: &std::io::Error) -> String {
    match err.raw_os_error() {
        Some(e) if e == libc::ESRCH => format!("no process {pid} is running"),
        Some(e) if e == libc::EPERM => {
            format!("not permitted to end {pid}; it belongs to another user")
        }
        _ => format!("could not signal {pid}: {err}"),
    }
}

// ── Where a reading came from ───────────────────────────────────────

/// Where the numbers on screen came from, and what is wrong if it was not the service.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Provenance {
    pub source: Source,
    /// Why the service was not used. `None` when it was.
    pub reason: Option<String>,
}

impl Provenance {
    pub fn service() -> Self {
        Self { source: Source::Service, reason: None }
    }

    pub fn local(reason: impl Into<String>) -> Self {
        Self { source: Source::Local, reason: Some(reason.into()) }
    }

    pub fn degraded(&self) -> bool {
        self.source == Source::Local
    }

    /// What the window says about a degraded reading, or nothing when there is nothing to say.
    ///
    /// The fallback is worth having — a monitor that goes blank because a service died is worse
    /// than one reading its own `sysinfo` — but it is not the same reading, and an audit of this
    /// app recorded the fallback's blind spots as if they were measurements.
    pub fn notice(&self) -> Option<String> {
        let reason = self.reason.as_deref()?;
        Some(format!(
            "Readings are local: the system-monitor service did not answer ({reason})"
        ))
    }
}

/// Take the service's answer if there is one, and record it when there is not.
///
/// This replaces `snapshot_via_service().unwrap_or_else(|_| snapshot_local())`, which threw the
/// reason away along with the fact that a fallback had happened at all. A caller cannot take
/// this helper's value without also taking the note about where it came from.
pub fn reading<T>(
    from_service: Result<T, String>,
    local: impl FnOnce() -> T,
) -> (T, Provenance) {
    match from_service {
        Ok(value) => (value, Provenance::service()),
        Err(reason) => (local(), Provenance::local(reason)),
    }
}

/// The worse of two provenances, for a poll that reads two things from the same service.
///
/// One degraded reading makes the window degraded; reporting `source: "service"` because the
/// other half of the poll happened to succeed would be the same lie in smaller print.
pub fn worse(a: Provenance, b: Provenance) -> Provenance {
    if a.degraded() {
        a
    } else if b.degraded() {
        b
    } else {
        a
    }
}

/// A string field the app may never have measured.
///
/// `describe` answered `cpu_model: ""` and `interfaces[0].ip: ""`, and an empty string sitting
/// among measurements reads as one — a CPU whose model is the empty string. Neither the service
/// nor the local fallback measures either of those today, and JSON already has a word for that.
pub fn measured(value: &str) -> serde_json::Value {
    if value.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(value.to_string())
    }
}

/// The one line the window shows, from the two things that can be wrong at the same time.
///
/// A failed kill and a dead service are independent, and the poll refreshes twice a second
/// faster than anyone reads — so a degraded-readings notice written every tick would wipe the
/// reason a kill failed before the person saw it. Both are kept, and both are shown.
pub fn compose_notice(failure: Option<&str>, degraded: Option<&str>) -> String {
    match (failure, degraded) {
        (Some(f), Some(d)) => format!("{f} — {d}"),
        (Some(f), None) => f.to_string(),
        (None, Some(d)) => d.to_string(),
        (None, None) => String::new(),
    }
}
