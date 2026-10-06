//! Every agent's commands: starting them, waiting on them, typing into them, stopping them — and
//! refusing any of that for a job that is another agent's.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::job::{self, Job};
use crate::AgentId;

/// How long `run` and `job` wait when the caller does not say: the design's two minutes.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(120);

/// The longest command line accepted, well under Linux's 128 KiB limit on one argument.
const COMMAND_LIMIT: usize = 100 * 1024;

/// The raw terminal bytes of one job, as they arrive: `(agent, job, bytes)`. Called on the job's
/// own worker thread — a view that draws has to hop to its UI thread.
pub type OutputSink = Arc<dyn Fn(&AgentId, &JobId, &[u8]) + Send + Sync>;

/// A job has ended: the answer an agent would get for it now. Called on the job's worker thread.
pub type FinishSink = Arc<dyn Fn(&RunAnswer) + Send + Sync>;

/// A job's id: `job-` and 128 random bits in hex. Unguessable, and meaningful only together with
/// the agent that started it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub String);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl JobId {
    fn random() -> JobId {
        let mut bytes = [0u8; 16];
        let mut filled = 0;
        while filled < bytes.len() {
            let n = unsafe {
                libc::getrandom(bytes[filled..].as_mut_ptr().cast(), bytes.len() - filled, 0)
            };
            if n > 0 {
                filled += n as usize;
            } else if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                use std::io::Read;
                std::fs::File::open("/dev/urandom")
                    .and_then(|mut f| f.read_exact(&mut bytes))
                    .expect("this machine has no source of randomness for job ids");
                break;
            }
        }
        JobId(format!("job-{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()))
    }
}

/// The bounds on what the agent terminal will do. [`Limits::default`] is the design's numbers.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Commands one agent may have running at once.
    pub per_agent: usize,
    /// Commands running on the desktop at once, across every agent.
    pub total: usize,
    /// Output kept per job for the view: half from the start, half from the end.
    pub retained_bytes: usize,
    /// The answer's tail: at most this many lines…
    pub tail_lines: usize,
    /// …and at most this many bytes of them.
    pub tail_bytes: usize,
    /// Quiet for this long while reading its terminal = waiting for input.
    pub silence: Duration,
    /// Between `SIGTERM` and `SIGKILL`.
    pub kill_grace: Duration,
    /// The longest a caller may wait in one call.
    pub max_wait: Duration,
    /// The PTY's size until a view resizes it.
    pub rows: u16,
    pub cols: u16,
    /// Rows of scrollback the emulator keeps (the tail is read from these).
    pub scrollback: usize,
    /// Finished jobs kept per agent for a late `job` call, oldest dropped first.
    pub keep_finished: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            per_agent: 4,
            total: 16,
            retained_bytes: 2 * 1024 * 1024,
            tail_lines: 200,
            tail_bytes: 16 * 1024,
            silence: Duration::from_secs(20),
            kill_grace: Duration::from_secs(2),
            max_wait: Duration::from_secs(600),
            rows: 24,
            cols: 120,
            scrollback: 300,
            keep_finished: 8,
        }
    }
}

/// The whole environment a command starts with. Built, never inherited: nothing from the shell's
/// or the harness's own environment reaches a command, so a variable a harness was started with —
/// a key, a token — cannot leak into an agent's `env`.
#[derive(Clone, Debug)]
pub struct Environment {
    pub home: PathBuf,
    pub user: String,
    pub path: String,
    /// The person's locale. The one value read from the shell's own environment, because a
    /// locale has to come from somewhere and it is not a secret.
    pub lang: String,
}

impl Environment {
    /// This user's, from the password database: `HOME` and `USER` as the system knows them, a
    /// fixed `PATH`, and the shell's `LANG` (or `C.UTF-8`).
    pub fn for_this_user() -> Environment {
        let (home, user) = passwd_entry().unwrap_or_else(|| {
            (
                std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")),
                std::env::var("USER").unwrap_or_else(|_| unsafe { libc::getuid() }.to_string()),
            )
        });
        let path = format!(
            "{}/.local/bin:/opt/yantrik/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            home.display()
        );
        let lang = std::env::var("LANG")
            .ok()
            .filter(|l| !l.is_empty() && !l.chars().any(char::is_control))
            .unwrap_or_else(|| "C.UTF-8".to_string());
        Environment { home, user, path, lang }
    }

    /// The variables, for a command starting in `cwd`.
    pub fn vars(&self, cwd: &Path) -> Vec<(String, String)> {
        vec![
            ("HOME".into(), self.home.display().to_string()),
            ("USER".into(), self.user.clone()),
            ("PATH".into(), self.path.clone()),
            ("LANG".into(), self.lang.clone()),
            ("TERM".into(), "xterm-256color".into()),
            ("PWD".into(), cwd.display().to_string()),
        ]
    }
}

fn passwd_entry() -> Option<(PathBuf, String)> {
    use std::ffi::CStr;
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    let rc = unsafe {
        libc::getpwuid_r(libc::getuid(), &mut entry, buf.as_mut_ptr(), buf.len(), &mut found)
    };
    if rc != 0 || found.is_null() || entry.pw_dir.is_null() || entry.pw_name.is_null() {
        return None;
    }
    let home = unsafe { CStr::from_ptr(entry.pw_dir) }.to_string_lossy().to_string();
    let user = unsafe { CStr::from_ptr(entry.pw_name) }.to_string_lossy().to_string();
    Some((PathBuf::from(home), user))
}

/// Where a job stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Running { waiting_for_input: bool },
    /// The command's shell exited with this status (`exit 3`, or the last command's status).
    Exited { code: i32 },
    /// The command's shell was killed by this signal.
    Signalled { signal: i32 },
}

/// What an agent is told about one of its commands.
#[derive(Clone, Debug, PartialEq)]
pub struct RunAnswer {
    pub job: JobId,
    pub agent: AgentId,
    pub command: String,
    /// Where it started.
    pub cwd: PathBuf,
    /// Where it ended, once it has: the directory its next command starts in, unless a later one
    /// finishes first. The start directory when the command reported none.
    pub cwd_after: Option<PathBuf>,
    pub state: JobState,
    /// Stopped by [`Jobs::kill`], [`Jobs::kill_agent`] or the shell closing.
    pub killed: bool,
    /// Running time so far, or until it ended.
    pub elapsed: Duration,
    /// The end of what the terminal shows, as text.
    pub tail: String,
    /// Some of the output is not in `tail`.
    pub tail_clipped: bool,
    /// Every byte the command wrote.
    pub output_bytes: u64,
    /// Bytes no longer kept for the view, past [`Limits::retained_bytes`].
    pub truncated_bytes: u64,
}

impl RunAnswer {
    pub fn running(&self) -> bool {
        matches!(self.state, JobState::Running { .. })
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self.state {
            JobState::Exited { code } => Some(code),
            _ => None,
        }
    }

    pub fn signal(&self) -> Option<i32> {
        match self.state {
            JobState::Signalled { signal } => Some(signal),
            _ => None,
        }
    }

    /// The answer as the shell's actions give it.
    pub fn to_json(&self) -> serde_json::Value {
        let mut out = serde_json::json!({
            "job": self.job,
            "agent": self.agent,
            "command": self.command,
            "running": self.running(),
            "cwd": self.cwd,
            "elapsed_ms": self.elapsed.as_millis() as u64,
            "tail": self.tail,
            "tail_clipped": self.tail_clipped,
            "output_bytes": self.output_bytes,
            "truncated_bytes": self.truncated_bytes,
        });
        match self.state {
            JobState::Running { waiting_for_input } => {
                out["waiting_for_input"] = waiting_for_input.into();
            }
            JobState::Exited { code } => {
                out["exit_code"] = code.into();
            }
            JobState::Signalled { signal } => {
                out["signal"] = signal.into();
                out["signal_name"] = signal_name(signal).into();
            }
        }
        if let Some(dir) = &self.cwd_after {
            out["cwd_after"] = serde_json::json!(dir);
        }
        if self.killed {
            out["killed"] = true.into();
        }
        out
    }
}

/// `SIGTERM` for 15, and so on for the signals a command is likely to die of.
pub fn signal_name(signal: i32) -> String {
    let name = match signal {
        libc::SIGHUP => "SIGHUP",
        libc::SIGINT => "SIGINT",
        libc::SIGQUIT => "SIGQUIT",
        libc::SIGILL => "SIGILL",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        libc::SIGFPE => "SIGFPE",
        libc::SIGKILL => "SIGKILL",
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGPIPE => "SIGPIPE",
        libc::SIGALRM => "SIGALRM",
        libc::SIGTERM => "SIGTERM",
        other => return format!("signal {other}"),
    };
    name.to_string()
}

/// One line per job, for `describe` and for the view's list.
#[derive(Clone, Debug, PartialEq)]
pub struct JobSummary {
    pub job: JobId,
    pub agent: AgentId,
    pub command: String,
    pub state: JobState,
    pub elapsed: Duration,
}

pub(crate) struct Shared {
    pub limits: Limits,
    /// Each agent's directory, and the start order of the command that reported it.
    pub dirs: Mutex<HashMap<AgentId, (PathBuf, u64)>>,
    /// Every listener for a job's bytes, in the order they were added.
    pub on_output: RwLock<Vec<OutputSink>>,
    /// Every listener for a job's end, in the order they were added.
    pub on_finish: RwLock<Vec<FinishSink>>,
}

struct Inner {
    shared: Arc<Shared>,
    env: Environment,
    jobs: Mutex<HashMap<JobId, Arc<Job>>>,
    next_seq: AtomicU64,
}

impl Drop for Inner {
    fn drop(&mut self) {
        shutdown(&self.jobs, &self.shared.limits);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Every agent's commands. Cheap to clone; the last clone to go kills whatever is still running.
#[derive(Clone)]
pub struct Jobs {
    inner: Arc<Inner>,
}

impl Jobs {
    pub fn new(limits: Limits) -> Jobs {
        Jobs::with_environment(limits, Environment::for_this_user())
    }

    pub fn with_environment(limits: Limits, env: Environment) -> Jobs {
        Jobs {
            inner: Arc::new(Inner {
                shared: Arc::new(Shared {
                    limits,
                    dirs: Mutex::new(HashMap::new()),
                    on_output: RwLock::new(Vec::new()),
                    on_finish: RwLock::new(Vec::new()),
                }),
                env,
                jobs: Mutex::new(HashMap::new()),
                next_seq: AtomicU64::new(1),
            }),
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.inner.shared.limits
    }

    /// Hand every job's raw terminal bytes to `sink` as they arrive, as well as to every sink added
    /// before it.
    ///
    /// Added to, not replaced, for [`Jobs::on_finish`]'s reason: the Agents store draws a command's
    /// card from these bytes, and anything else that listens — a popped-out view, a test — must not
    /// silently take them away from it depending on which was wired last.
    pub fn on_output(&self, sink: impl Fn(&AgentId, &JobId, &[u8]) + Send + Sync + 'static) {
        self.inner.shared.on_output.write().unwrap_or_else(|e| e.into_inner()).push(Arc::new(sink));
    }

    /// Tell `sink` whenever a job ends, as well as every sink added before it.
    ///
    /// Added to, not replaced, because more than one part of the shell has to hear of an end: the
    /// Agents view settles the command's card, and the bridge glue tells the agent, in its next
    /// turn, about a command that finished after its call had returned. Either one replacing the
    /// other would be a silent loss depending on which was wired first.
    pub fn on_finish(&self, sink: impl Fn(&RunAnswer) + Send + Sync + 'static) {
        self.inner.shared.on_finish.write().unwrap_or_else(|e| e.into_inner()).push(Arc::new(sink));
    }

    /// The directory `agent`'s next command starts in: where its latest command ended, or home.
    pub fn directory(&self, agent: &AgentId) -> PathBuf {
        lock(&self.inner.shared.dirs)
            .get(agent)
            .map(|(dir, _)| dir.clone())
            .filter(|dir| dir.is_dir())
            .unwrap_or_else(|| self.inner.env.home.clone())
    }

    /// Start `command` for `agent` and wait up to `wait` for it to end (or to start waiting for
    /// input). The answer says which.
    pub fn run(
        &self,
        agent: &AgentId,
        command: &str,
        cwd: Option<PathBuf>,
        wait: Duration,
    ) -> Result<RunAnswer, String> {
        let job = self.start(agent, command, cwd)?;
        self.job(agent, &job, wait)
    }

    /// Start `command` for `agent` without waiting. `cwd`, when given, is taken relative to the
    /// agent's directory and must exist.
    pub fn start(&self, agent: &AgentId, command: &str, cwd: Option<PathBuf>) -> Result<JobId, String> {
        let limits = &self.inner.shared.limits;
        if command.trim().is_empty() {
            return Err("`command` is empty: one command line, as it would be typed.".to_string());
        }
        if command.len() > COMMAND_LIMIT {
            return Err(format!(
                "`command` is {} KiB; a command line is at most {} KiB. Put a long script in a file \
                 and run the file.",
                command.len() / 1024,
                COMMAND_LIMIT / 1024
            ));
        }
        if command.contains('\0') {
            return Err("`command` contains a NUL byte, which no command line can carry.".to_string());
        }
        let here = self.directory(agent);
        let cwd = match cwd {
            None => here,
            Some(dir) => {
                let dir = if dir.is_absolute() { dir } else { here.join(dir) };
                if !dir.is_dir() {
                    return Err(format!("`cwd` {} is not a directory on this machine.", dir.display()));
                }
                dir
            }
        };

        let mut jobs = lock(&self.inner.jobs);
        let running: Vec<&Arc<Job>> = jobs.values().filter(|j| j.running()).collect();
        let mine: Vec<String> =
            running.iter().filter(|j| &j.agent == agent).map(|j| j.id.to_string()).collect();
        if mine.len() >= limits.per_agent {
            return Err(format!(
                "{agent} already has {} commands running ({}), the most one agent may run at once. \
                 Wait for one with `agent_job` or stop one with `agent_kill`.",
                mine.len(),
                mine.join(", ")
            ));
        }
        if running.len() >= limits.total {
            return Err(format!(
                "{} commands are running on this desktop across all agents, the most it runs at \
                 once. Try again when one has finished.",
                running.len()
            ));
        }

        // Finished jobs are kept for a late `agent_job`, but not forever.
        let mut finished: Vec<(u64, JobId)> = jobs
            .values()
            .filter(|j| &j.agent == agent && !j.running())
            .map(|j| (j.seq, j.id.clone()))
            .collect();
        finished.sort();
        let excess = (finished.len() + 1).saturating_sub(limits.keep_finished.max(1));
        for (_, id) in finished.into_iter().take(excess) {
            jobs.remove(&id);
        }

        let seq = self.inner.next_seq.fetch_add(1, Ordering::Relaxed);
        let id = JobId::random();
        let env = self.inner.env.vars(&cwd);
        let job = job::spawn(id.clone(), agent.clone(), command, cwd, seq, &env, self.inner.shared.clone())?;
        tracing::info!(agent = %agent, job = %id, pid = job.pid, "agent command started");
        jobs.insert(id.clone(), job);
        Ok(id)
    }

    /// Wait up to `wait` (at most [`Limits::max_wait`]) for one of `agent`'s jobs to end, and
    /// answer with where it stands. `Duration::ZERO` only looks.
    pub fn job(&self, agent: &AgentId, job: &JobId, wait: Duration) -> Result<RunAnswer, String> {
        let job = self.find(agent, job)?;
        job.wait(wait.min(self.inner.shared.limits.max_wait));
        Ok(job.answer(&self.inner.shared.limits))
    }

    /// Type `text` into one of `agent`'s jobs, exactly as given: no newline is added.
    pub fn input(&self, agent: &AgentId, job: &JobId, text: &str) -> Result<(), String> {
        self.find(agent, job)?.write(text.as_bytes())
    }

    /// Stop one of `agent`'s jobs: `SIGTERM` to its whole process group now, `SIGKILL` after
    /// [`Limits::kill_grace`]. `Ok(false)` when it had already ended.
    pub fn kill(&self, agent: &AgentId, job: &JobId) -> Result<bool, String> {
        Ok(self.find(agent, job)?.terminate())
    }

    /// Stop every running job of `agent` — Stop on its pane, or its row closed. The ids stopped.
    pub fn kill_agent(&self, agent: &AgentId) -> Vec<JobId> {
        let jobs: Vec<Arc<Job>> =
            lock(&self.inner.jobs).values().filter(|j| &j.agent == agent).cloned().collect();
        jobs.into_iter().filter(|j| j.terminate()).map(|j| j.id.clone()).collect()
    }

    /// Every job still held, oldest first.
    pub fn list(&self) -> Vec<JobSummary> {
        let mut jobs: Vec<Arc<Job>> = lock(&self.inner.jobs).values().cloned().collect();
        jobs.sort_by_key(|j| j.seq);
        jobs.iter()
            .map(|j| {
                let (state, elapsed) = j.status();
                JobSummary {
                    job: j.id.clone(),
                    agent: j.agent.clone(),
                    command: j.command.clone(),
                    state,
                    elapsed,
                }
            })
            .collect()
    }

    /// The pid of every command still running: each is its `bash`, the session leader of
    /// everything the command started. The shell refuses a run stamp from any process with one
    /// of these in its ancestry, because those processes run as the person on an agent's word.
    pub fn live_pids(&self) -> Vec<i32> {
        lock(&self.inner.jobs)
            .values()
            .filter(|j| matches!(j.status().0, JobState::Running { .. }))
            .map(|j| j.pid)
            .collect()
    }

    // ── The view's side: the shell's own, so no agent is asked for ──

    /// Whose job this is.
    pub fn owner(&self, job: &JobId) -> Option<AgentId> {
        lock(&self.inner.jobs).get(job).map(|j| j.agent.clone())
    }

    /// Where a job stands, for the view.
    pub fn answer(&self, job: &JobId) -> Option<RunAnswer> {
        let job = lock(&self.inner.jobs).get(job).cloned()?;
        Some(job.answer(&self.inner.shared.limits))
    }

    /// The person typing into a job's card. Goes to the command and nowhere else.
    pub fn type_input(&self, job: &JobId, bytes: &[u8]) -> Result<(), String> {
        let job = lock(&self.inner.jobs).get(job).cloned().ok_or("that job is gone")?;
        job.write(bytes)
    }

    /// Resize a job's terminal to the card drawing it.
    pub fn resize(&self, job: &JobId, rows: u16, cols: u16) {
        if let Some(job) = lock(&self.inner.jobs).get(job).cloned() {
            job.resize(rows, cols);
        }
    }

    /// Read a job's screen, to draw it.
    pub fn with_screen<R>(&self, job: &JobId, f: impl FnOnce(&vt100::Screen) -> R) -> Option<R> {
        let job = lock(&self.inner.jobs).get(job).cloned()?;
        Some(job.with_screen(f))
    }

    /// Everything kept of a job's raw output: the head, a marker, the tail.
    pub fn output(&self, job: &JobId) -> Option<Vec<u8>> {
        let job = lock(&self.inner.jobs).get(job).cloned()?;
        Some(job.output())
    }

    /// Kill every running command, as the shell closes. Also what dropping the last `Jobs` does.
    pub fn shutdown(&self) {
        shutdown(&self.inner.jobs, &self.inner.shared.limits);
    }

    fn find(&self, agent: &AgentId, job: &JobId) -> Result<Arc<Job>, String> {
        match lock(&self.inner.jobs).get(job) {
            None => Err(format!(
                "no job `{job}` on this desktop. Job ids come from `agent_run`, and a finished job \
                 is kept only until newer ones replace it."
            )),
            Some(found) if &found.agent != agent => Err(format!(
                "job `{job}` belongs to another agent. An agent can wait on, type into and stop only \
                 its own commands."
            )),
            Some(found) => Ok(found.clone()),
        }
    }
}

/// `SIGTERM` to every running group, up to the grace period for them to go, then `SIGKILL` to
/// whatever is left — including the stragglers of groups whose leader has already gone.
fn shutdown(jobs: &Mutex<HashMap<JobId, Arc<Job>>>, limits: &Limits) {
    let all: Vec<Arc<Job>> = lock(jobs).values().cloned().collect();
    let stopping: Vec<Arc<Job>> = all.into_iter().filter(|j| j.terminate()).collect();
    if stopping.is_empty() {
        return;
    }
    tracing::info!(jobs = stopping.len(), "stopping agent commands");
    let deadline = Instant::now() + limits.kill_grace;
    while Instant::now() < deadline && stopping.iter().any(|j| j.running()) {
        std::thread::sleep(Duration::from_millis(20));
    }
    for job in &stopping {
        job.kill_now();
    }
}
