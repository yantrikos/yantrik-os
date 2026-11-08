//! One command in one PTY: the spawn, and the worker that owns the terminal until it is over.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::jobs::{JobId, JobState, Limits, RunAnswer, Shared};
use crate::retained::Retained;
use crate::AgentId;

/// What `bash -c` runs. `$1` is the command.
///
/// The `EXIT` trap is what makes the directory the command's own account: it runs on the way out
/// whether the command fell off its end or called `exit 3`, and writes `$PWD`, NUL-terminated, to
/// fd 3 — a pipe only the shell reads. It keeps the command's status. `eval` runs the command in
/// this same shell, so its `cd` is this shell's `cd`. The trailing `exit` keeps `eval` off the end
/// of the `-c` string, where bash has been growing its habit of replacing itself with the last
/// program instead of forking it. Bash 5.2 does not do that while an `EXIT` trap is set (checked:
/// `cd / && ls` reports `/` with or without this line), so it is a guard against a later bash, not
/// a fix for this one.
pub(crate) const WRAPPER: &str = r#"trap '__yantrik_status=$?; builtin printf "%s\0" "$PWD" >&3 2>/dev/null; exit "$__yantrik_status"' EXIT
__yantrik_run=$1
set --
eval "$__yantrik_run"
exit "$?"
"#;

/// Bytes that may wait to be written to one job's terminal.
pub(crate) const INPUT_LIMIT: usize = 64 * 1024;

/// How often the worker looks up from the terminal when nothing is happening: the resolution of
/// exit detection, the kill grace and the silence clock.
const TICK_MS: i32 = 100;

/// How the command's shell ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    Exited(i32),
    Signalled(i32),
}

#[derive(Default)]
struct State {
    end: Option<End>,
    cwd_after: Option<PathBuf>,
    ended_after: Option<Duration>,
    kill_requested: Option<Instant>,
    reaped: bool,
}

pub(crate) struct Job {
    pub id: JobId,
    pub agent: AgentId,
    pub command: String,
    pub cwd: PathBuf,
    /// Start order across all agents; decides whose directory wins (see the crate doc).
    pub seq: u64,
    pub started: Instant,
    /// The command's `bash`: session leader, so also the process group's id.
    pub pid: i32,
    /// The job's systemd scope, when it has one: stopped with the job, to reach what left the group.
    scope: Option<String>,
    /// `/dev/pts/N`, for telling a read on this terminal from any other read.
    tty: PathBuf,
    state: Mutex<State>,
    changed: Condvar,
    screen: Mutex<vt100::Parser>,
    output: Mutex<Retained>,
    /// Last output or input: the silence clock.
    activity: Mutex<Instant>,
    waiting: AtomicBool,
    pending_input: Mutex<Vec<u8>>,
    pending_size: Mutex<Option<(u16, u16)>>,
    wake: UnixStream,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Start `command` in a fresh PTY, in `cwd`, with exactly `env`.
pub(crate) fn spawn(
    id: JobId,
    agent: AgentId,
    command: &str,
    cwd: PathBuf,
    seq: u64,
    env: &[(String, String)],
    shared: Arc<Shared>,
) -> Result<Arc<Job>, String> {
    let limits = &shared.limits;
    let refuse = |what: &str, e: &dyn std::fmt::Display| format!("the command was not started: {what} ({e})");

    let (pty, pts) = pty_process::blocking::open().map_err(|e| refuse("no terminal could be opened", &e))?;
    pty.resize(pty_process::Size::new(limits.rows, limits.cols))
        .map_err(|e| refuse("the terminal could not be sized", &e))?;

    let (pwd_read, pwd_write) = pipe().map_err(|e| refuse("no pipe for the directory", &e))?;
    let (wake, wake_reader) = UnixStream::pair().map_err(|e| refuse("no wake socket", &e))?;
    wake.set_nonblocking(true).map_err(|e| refuse("no wake socket", &e))?;
    wake_reader.set_nonblocking(true).map_err(|e| refuse("no wake socket", &e))?;

    let bash = ["/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .find(|p| Path::new(p).exists())
        .ok_or_else(|| "the command was not started: this machine has no bash".to_string())?;
    let write_fd = pwd_write.as_raw_fd();
    // In a scope of its own, outside the person's session (#1137); see `scope`.
    let launch = crate::scope::launch(&id.0, bash);
    let builder = pty_process::blocking::Command::new(&launch.program)
        .args(&launch.args)
        .args(["--noprofile", "--norc", "-c", WRAPPER, "bash", command])
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .envs(launch.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(&cwd);
    // Safety: only async-signal-safe calls (dup2, fcntl) between fork and exec.
    let builder = unsafe { builder.pre_exec(move || hand_over_pwd_pipe(write_fd)) };
    let child = builder.spawn(pts).map_err(|e| refuse("bash would not start", &e))?;
    // Ours closed, so the command's copy is the only writer.
    drop(pwd_write);

    let pid = child.id() as i32;
    let tty = std::fs::read_link(format!("/proc/{pid}/fd/0")).unwrap_or_default();
    let master = File::from(OwnedFd::from(pty));
    let raw = master.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(raw, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }

    let job = Arc::new(Job {
        id,
        agent,
        command: command.to_string(),
        cwd,
        seq,
        started: Instant::now(),
        pid,
        scope: launch.unit,
        tty,
        state: Mutex::new(State::default()),
        changed: Condvar::new(),
        screen: Mutex::new(vt100::Parser::new(limits.rows, limits.cols, limits.scrollback)),
        output: Mutex::new(Retained::new(limits.retained_bytes)),
        activity: Mutex::new(Instant::now()),
        waiting: AtomicBool::new(false),
        pending_input: Mutex::new(Vec::new()),
        pending_size: Mutex::new(None),
        wake,
    });

    let worker_job = job.clone();
    let started = std::thread::Builder::new()
        .name(format!("agent-job-{pid}"))
        .spawn(move || worker(worker_job, master, pwd_read, wake_reader, child, shared));
    if let Err(e) = started {
        // Nothing would ever read its terminal or reap it: take it down now rather than leave a
        // command running that nobody can see.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        if let Some(unit) = &job.scope {
            crate::scope::stop(unit);
        }
        return Err(refuse("no thread to watch it", &e));
    }
    Ok(job)
}

/// In the child, between fork and exec: put the directory pipe on fd 3, open across exec.
fn hand_over_pwd_pipe(fd: RawFd) -> std::io::Result<()> {
    unsafe {
        if fd == 3 {
            let flags = libc::fcntl(3, libc::F_GETFD);
            if flags < 0 || libc::fcntl(3, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        } else if libc::dup2(fd, 3) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as RawFd; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Whether the leader has ended, without reaping it.
///
/// `WNOWAIT` is the point: while the leader is an unreaped zombie its pid — which is the group's
/// id — cannot be handed to anyone else, so a `SIGKILL` sent to the group after the grace period
/// can only reach this job's processes.
fn peek_exit(pid: i32) -> Option<End> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let done = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if done != 0 {
        // ECHILD: something else reaped it. The command is over and how it ended is lost.
        return (std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD))
            .then_some(End::Exited(-1));
    }
    if unsafe { info.si_pid() } == 0 {
        return None;
    }
    let status = unsafe { info.si_status() };
    match info.si_code {
        libc::CLD_EXITED => Some(End::Exited(status)),
        libc::CLD_KILLED | libc::CLD_DUMPED => Some(End::Signalled(status)),
        _ => None,
    }
}

/// The last complete directory the wrapper wrote, if it is still a directory.
fn reported_directory(pipe: &OwnedFd) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let raw = pipe.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(raw, libc::F_GETFL);
        libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
    let mut file = File::from(pipe.try_clone().ok()?);
    let mut bytes = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = file.read(&mut buf) {
        if n == 0 || bytes.len() > 64 * 1024 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    // Only NUL-terminated records count; take the last. Anything after the last NUL is a record
    // cut short and says nothing.
    let complete = &bytes[..bytes.iter().rposition(|b| *b == 0)?];
    let last = complete.rsplit(|b| *b == 0).next()?;
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(last));
    (path.is_absolute() && path.is_dir()).then_some(path)
}

/// Every row the emulator still holds — scrollback, then the screen — with wrapped rows joined
/// back into the lines the command wrote, and the empty rows under the output left off.
fn rendered_lines(parser: &mut vt100::Parser) -> Vec<String> {
    let saved = parser.screen().scrollback();
    let (rows, cols) = parser.screen().size();
    let mut lines: Vec<String> = Vec::new();
    let mut joining = false;
    let mut push = |text: String, wrapped: bool| {
        match lines.last_mut() {
            Some(last) if joining => last.push_str(&text),
            _ => lines.push(text),
        }
        joining = wrapped;
    };
    parser.screen_mut().set_scrollback(usize::MAX);
    let mut remaining = parser.screen().scrollback();
    while remaining > 0 {
        let take = remaining.min(rows as usize);
        let screen = parser.screen();
        for (row, text) in screen.rows(0, cols).take(take).enumerate() {
            push(text, screen.row_wrapped(row as u16));
        }
        remaining -= take;
        parser.screen_mut().set_scrollback(remaining);
    }
    let screen = parser.screen();
    for (row, text) in screen.rows(0, cols).enumerate() {
        push(text, screen.row_wrapped(row as u16));
    }
    parser.screen_mut().set_scrollback(saved);
    let mut lines: Vec<String> = lines.into_iter().map(|l| l.trim_end().to_string()).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// The last `max_lines` lines, at most `max_bytes`, and whether anything was left out.
pub(crate) fn tail_of(parser: &mut vt100::Parser, max_lines: usize, max_bytes: usize) -> (String, bool) {
    let lines = rendered_lines(parser);
    let from = lines.len().saturating_sub(max_lines);
    let mut tail = lines[from..].join("\n");
    let mut clipped = from > 0;
    if tail.len() > max_bytes {
        let mut cut = tail.len() - max_bytes;
        while !tail.is_char_boundary(cut) {
            cut += 1;
        }
        tail = tail[cut..].to_string();
        clipped = true;
    }
    (tail, clipped)
}

impl Job {
    /// Where the job stands and for how long, without reading its screen: what a list needs.
    pub(crate) fn status(&self) -> (JobState, Duration) {
        let state = lock(&self.state);
        let now = match state.end {
            None => JobState::Running { waiting_for_input: self.waiting.load(Ordering::Acquire) },
            Some(End::Exited(code)) => JobState::Exited { code },
            Some(End::Signalled(signal)) => JobState::Signalled { signal },
        };
        (now, state.ended_after.unwrap_or_else(|| self.started.elapsed()))
    }

    /// Where the job stands, with the tail of what it printed.
    pub(crate) fn answer(&self, limits: &Limits) -> RunAnswer {
        let (state, elapsed) = self.status();
        let (cwd_after, killed) = {
            let state = lock(&self.state);
            (state.cwd_after.clone(), state.kill_requested.is_some())
        };
        let (tail, tail_clipped) = tail_of(&mut lock(&self.screen), limits.tail_lines, limits.tail_bytes);
        let (output_bytes, truncated_bytes) = {
            let out = lock(&self.output);
            (out.total(), out.dropped())
        };
        RunAnswer {
            job: self.id.clone(),
            agent: self.agent.clone(),
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            cwd_after,
            state,
            killed,
            elapsed,
            tail,
            tail_clipped,
            output_bytes,
            truncated_bytes,
        }
    }

    pub(crate) fn running(&self) -> bool {
        lock(&self.state).end.is_none()
    }

    /// Block until the command ends, starts waiting for input, or `timeout` passes.
    pub(crate) fn wait(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.state);
        loop {
            if state.end.is_some() || self.waiting.load(Ordering::Acquire) {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            state = self
                .changed
                .wait_timeout(state, deadline - now)
                .map(|(guard, _)| guard)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    /// Queue bytes for the terminal, exactly as given.
    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), String> {
        if !self.running() {
            return Err(format!("job `{}` has finished; there is nothing reading its terminal.", self.id));
        }
        if bytes.len() > INPUT_LIMIT {
            return Err(format!("input is limited to {} KiB per call.", INPUT_LIMIT / 1024));
        }
        {
            let mut pending = lock(&self.pending_input);
            if pending.len() + bytes.len() > INPUT_LIMIT {
                return Err(format!(
                    "job `{}` has not read the {} bytes it was already sent; try again when it has.",
                    self.id,
                    pending.len()
                ));
            }
            pending.extend_from_slice(bytes);
        }
        self.touch();
        let _ = (&self.wake).write(&[1]);
        Ok(())
    }

    pub(crate) fn resize(&self, rows: u16, cols: u16) {
        *lock(&self.pending_size) = Some((rows.clamp(2, 500), cols.clamp(8, 1000)));
        let _ = (&self.wake).write(&[1]);
    }

    pub(crate) fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        f(lock(&self.screen).screen())
    }

    pub(crate) fn output(&self) -> Vec<u8> {
        lock(&self.output).bytes()
    }

    /// `SIGTERM` to the whole group, now; the worker follows with `SIGKILL` after the grace
    /// period. `false` when the command had already ended.
    pub(crate) fn terminate(&self) -> bool {
        let mut state = lock(&self.state);
        if state.end.is_some() {
            return false;
        }
        if state.kill_requested.is_none() {
            // The leader is not reaped while `end` is unset, so `-pid` is still this group.
            unsafe {
                libc::kill(-self.pid, libc::SIGTERM);
                // A stopped process would sit on the TERM until continued.
                libc::kill(-self.pid, libc::SIGCONT);
            }
            state.kill_requested = Some(Instant::now());
        }
        drop(state);
        let _ = (&self.wake).write(&[1]);
        true
    }

    /// `SIGKILL` to the group, if its leader has not been reaped. For the shell's last moments,
    /// when no worker will be around to finish the grace period.
    pub(crate) fn kill_now(&self) {
        let state = lock(&self.state);
        if !state.reaped {
            unsafe {
                libc::kill(-self.pid, libc::SIGKILL);
            }
            if let Some(unit) = &self.scope {
                crate::scope::stop(unit);
            }
        }
    }

    fn touch(&self) {
        *lock(&self.activity) = Instant::now();
        if self.waiting.swap(false, Ordering::AcqRel) {
            let _state = lock(&self.state);
            self.changed.notify_all();
        }
    }

    fn feed(&self, bytes: &[u8], shared: &Shared) {
        lock(&self.screen).process(bytes);
        lock(&self.output).push(bytes);
        self.touch();
        let sinks = shared.on_output.read().unwrap_or_else(|e| e.into_inner()).clone();
        for sink in sinks {
            sink(&self.agent, &self.id, bytes);
        }
    }

    fn set_waiting(&self, waiting: bool) {
        if self.waiting.swap(waiting, Ordering::AcqRel) != waiting {
            let _state = lock(&self.state);
            self.changed.notify_all();
        }
    }
}

/// Read what the terminal has, in bounded batches. `false` once every slave end has closed.
fn read_available(master: &mut File, job: &Job, shared: &Shared, buf: &mut [u8]) -> bool {
    for _ in 0..16 {
        match master.read(buf) {
            Ok(0) => return false,
            Ok(n) => job.feed(&buf[..n], shared),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return true,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // EIO: Linux's answer once the last process holding the terminal has closed it.
            Err(_) => return false,
        }
    }
    true
}

fn poll_one(fd: RawFd, events: i16, timeout_ms: i32) -> i16 {
    let mut p = libc::pollfd { fd, events, revents: 0 };
    if unsafe { libc::poll(&mut p, 1, timeout_ms) } > 0 {
        p.revents
    } else {
        0
    }
}

/// Owns the terminal from spawn to reap.
fn worker(
    job: Arc<Job>,
    mut master: File,
    pwd: OwnedFd,
    mut wake: UnixStream,
    mut child: Child,
    shared: Arc<Shared>,
) {
    let limits = shared.limits.clone();
    let raw = master.as_raw_fd();
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 8192];
    let mut open = true;
    let mut escalated = false;
    let mut last_probe: Option<Instant> = None;

    loop {
        pending.append(&mut lock(&job.pending_input));
        if let Some((rows, cols)) = lock(&job.pending_size).take() {
            let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
            if open && unsafe { libc::ioctl(raw, libc::TIOCSWINSZ, &size) } == 0 {
                lock(&job.screen).screen_mut().set_size(rows, cols);
            }
        }

        let mut fds = [
            libc::pollfd {
                fd: if open { raw } else { -1 },
                events: libc::POLLIN | if pending.is_empty() { 0 } else { libc::POLLOUT },
                revents: 0,
            },
            libc::pollfd { fd: wake.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, TICK_MS) } < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
        {
            std::thread::sleep(Duration::from_millis(TICK_MS as u64));
        }
        if fds[1].revents != 0 {
            let mut sink = [0u8; 64];
            while wake.read(&mut sink).is_ok_and(|n| n > 0) {}
        }
        if open && !pending.is_empty() {
            match master.write(&pending) {
                Ok(n) => {
                    pending.drain(..n);
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => {}
                Err(_) => pending.clear(),
            }
        }
        if open && fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            open = read_available(&mut master, &job, &shared, &mut buf);
        }

        let (ended, kill_requested) = {
            let state = lock(&job.state);
            (state.end.is_some(), state.kill_requested)
        };

        if !ended {
            if let Some(end) = peek_exit(job.pid) {
                // What the command wrote just before it went can still be on its way through the
                // line discipline: read until the terminal closes or goes quiet.
                let deadline = Instant::now() + Duration::from_millis(500);
                while open && Instant::now() < deadline {
                    if poll_one(raw, libc::POLLIN, 30) == 0 {
                        break;
                    }
                    open = read_available(&mut master, &job, &shared, &mut buf);
                }
                let cwd_after = reported_directory(&pwd).unwrap_or_else(|| job.cwd.clone());
                {
                    let mut state = lock(&job.state);
                    state.end = Some(end);
                    state.cwd_after = Some(cwd_after.clone());
                    state.ended_after = Some(job.started.elapsed());
                    job.changed.notify_all();
                }
                job.waiting.store(false, Ordering::Release);
                tracing::info!(agent = %job.agent, job = %job.id, ?end, "agent command ended");
                {
                    let mut dirs = lock(&shared.dirs);
                    let newer = dirs.get(&job.agent).is_none_or(|(_, seq)| *seq < job.seq);
                    if newer {
                        dirs.insert(job.agent.clone(), (cwd_after, job.seq));
                    }
                }
                let sinks = shared.on_finish.read().unwrap_or_else(|e| e.into_inner()).clone();
                if !sinks.is_empty() {
                    let answer = job.answer(&limits);
                    for sink in sinks {
                        sink(&answer);
                    }
                }
                continue;
            }

            // Silence: only worth a look once the output has been quiet long enough, and then
            // once a second, not once a tick.
            let silent = lock(&job.activity).elapsed();
            if silent >= limits.silence {
                if last_probe.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
                    job.set_waiting(open && crate::proc::waiting_on_terminal(raw, job.pid, &job.tty));
                    last_probe = Some(Instant::now());
                }
            } else {
                last_probe = None;
            }
        }

        if let Some(asked) = kill_requested {
            if !escalated && asked.elapsed() >= limits.kill_grace {
                // Still unreaped, so still this group: whatever ignored the TERM goes now.
                unsafe {
                    libc::kill(-job.pid, libc::SIGKILL);
                }
                if let Some(unit) = &job.scope {
                    crate::scope::stop(unit);
                }
                escalated = true;
            }
        }

        if ended && (kill_requested.is_none() || escalated) {
            let _ = child.wait();
            lock(&job.state).reaped = true;
            break;
        }
    }
}
