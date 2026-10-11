//! An agent's commands, on the shell's surface: `agent_run`, `agent_job`, `agent_input`,
//! `agent_kill`.
//!
//! The terminal itself — a PTY per command, the directory that carries, the process groups, the
//! built environment, the caps — is `yantrik-agent-terminal`, tested without a shell. This module
//! is the door: it reads who is calling, turns the call's agent token into an agent, and hands the
//! work to the terminal off the UI thread. See `design/agents-workspace-2026-09-23.md`, decision 3.
//!
//! # Who the agent is
//!
//! Never an argument, and the token that says it is not one either. The token the host gave the
//! agent's harness with its first turn rides on `app.act` BESIDE `args` — `{action, args,
//! agent_token}`, the way a grant does — and reaches these handlers as `control::agent_token()`.
//! It is kept out of `args` because `args` is what gets shown and kept: the approval card draws
//! them, `record_unasked_action` writes them to `mind-audit.jsonl`, a grant is bound to them. The
//! runtime strips an `agent_token` a caller puts inside `args` anyway.
//!
//! The resolver checks the token against the kernel's account of the caller: the pid on the socket
//! has to descend from the harness that holds it. [`serve_host`] installs the shell's one —
//! [`HostTokens`], over the harness host — when the host is made (`wire/harness.rs`). Before that,
//! the resolver knows no tokens, so every call is answered "no agent holds this token": inert, but
//! a real answer.
//!
//! # A command that finishes after its call returned
//!
//! `agent_run` answers `running: true` when its `wait` runs out, and the agent goes on with its
//! turn. When that command ends, nobody's call carries the end — so the agent is told in its next
//! turn instead: the desktop leaves it a note (`Host::note_for`) that rides in that turn's context.
//! Which ends are owed is decided from what each agent was last *told*: a job whose last answer
//! said `running` is owed its end, unless the agent is waiting on it again (`agent_job`,
//! `agent_input`, `agent_kill`), in which case that call's answer carries it. See [`Late`].
//!
//! # Off the UI thread
//!
//! A command can take minutes and its caller is owed the exit code, so each handler only reads its
//! arguments, the caller and the token on the UI thread and hands the rest — resolving the token,
//! starting, waiting, killing — to `control::answer_later`, which finishes it on the socket's side.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use serde_json::{json, Value};
use yantrik_agent_terminal::{
    AgentId, AgentResolver, HostTokens, JobId, JobState, Jobs, Limits, NoAgents, RunAnswer,
    DEFAULT_WAIT,
};
use yantrik_app_runtime::control::{self, Action, App as ControlSurface, Param};

static JOBS: OnceLock<Jobs> = OnceLock::new();
static RESOLVER: RwLock<Option<Arc<dyn AgentResolver>>> = RwLock::new(None);

/// Where a late finish is reported: `(agent, note) → kept?`. The host's `note_for`, once
/// [`serve_host`] has run; until then a late finish is noticed and not reported.
type Deliver = Arc<dyn Fn(&AgentId, String) -> bool + Send + Sync>;
static DELIVER: RwLock<Option<Deliver>> = RwLock::new(None);

/// Every agent's commands. The Agents screen draws from this (`on_output`, `with_screen`,
/// `type_input`) — the same store the actions answer from, so a card and an answer cannot disagree.
pub fn jobs() -> &'static Jobs {
    JOBS.get_or_init(|| {
        let jobs = Jobs::new(Limits::default());
        // Every end is looked at for whether its agent is still owed it. Added when the store is
        // made, so no command can end before anybody is listening.
        jobs.on_finish(|answer| {
            let owed = late().finished(answer);
            if let Some(done) = owed {
                report_late(&done);
            }
        });
        // And every command is a verified card in its own agent's pane: its bytes as they come off
        // the PTY, and its end with the process's own exit code. Keyed by the job's owner — the
        // agent the terminal started it for — so one agent's output can never land in another's
        // card. On the job's worker thread; the store is behind a lock of its own, so no hop to
        // the UI thread is needed.
        jobs.on_output(|agent, job, bytes| {
            crate::agents::store().command_output(agent, &job.0, bytes);
            // And the same bytes to the terminal in Mind View, for an agent whose commands are shown.
            crate::mind_agent_terminal::mirror(&agent.0, bytes);
        });
        jobs.on_finish(|answer| {
            crate::agents::store().command_finished(
                &answer.agent,
                &answer.job.0,
                &answer.command,
                answer.exit_code(),
                answer.killed,
            )
        });
        jobs
    })
}

/// Tie the agent terminal to the harness host, once, when the shell makes the host: a token is
/// believed only as the host issued it and only from under the harness it was issued to, and a
/// command that finishes after its call returned is noted into its agent's next turn.
pub fn serve_host(host: &yantrik_harness::Host) {
    install_resolver(Arc::new(HostTokens::new(host.clone())));
    let host = host.clone();
    report_late_finishes(move |agent, note| host.note_for(agent, note));
}

/// Where tokens come from. [`serve_host`] installs the shell's; a test installs its own.
pub fn install_resolver(resolver: Arc<dyn AgentResolver>) {
    *RESOLVER.write().unwrap_or_else(|e| e.into_inner()) = Some(resolver);
}

/// Where a command that finished after its call returned is reported. Replaces any earlier one.
pub fn report_late_finishes(deliver: impl Fn(&AgentId, String) -> bool + Send + Sync + 'static) {
    *DELIVER.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(deliver));
}

/// The one record of which ends are owed.
fn late() -> std::sync::MutexGuard<'static, Late> {
    static LATE: OnceLock<Mutex<Late>> = OnceLock::new();
    LATE.get_or_init(|| Mutex::new(Late::default())).lock().unwrap_or_else(|e| e.into_inner())
}

/// An agent's call on `job` has begun; whatever it answers decides afresh whether the end is owed.
fn engage(agent: &AgentId, job: &JobId) {
    late().engage(agent, job);
}

/// The agent has just been given `answer`.
fn told(answer: &RunAnswer) {
    let owed = late().told(answer);
    if let Some(done) = owed {
        report_late(&done);
    }
}

fn report_late(done: &RunAnswer) {
    let deliver = DELIVER.read().unwrap_or_else(|e| e.into_inner()).clone();
    let Some(deliver) = deliver else {
        tracing::debug!(agent = %done.agent, job = %done.job, "a command finished late; nothing to report it to");
        return;
    };
    let kept = deliver(&done.agent, late_note(done));
    tracing::info!(agent = %done.agent, job = %done.job, kept, "a command finished after its call returned; noted for the agent's next turn");
}

/// Which of the agents' commands were last reported to their agent as still running — and so,
/// when one ends, whose end is owed to the agent.
///
/// Three moments, all under one lock, because they race: an agent's call begins on a job
/// ([`Late::engage`]), an answer is given to it ([`Late::told`]), and a job ends
/// ([`Late::finished`], on the job's own thread). A job can end in the instant between its
/// answer being read as `running` and that answer reaching this record; `ended` keeps the last
/// few ends nobody was owed so that case is still caught.
#[derive(Default)]
struct Late {
    /// Jobs whose agent's last word on them was `running`, and whose they are.
    owed: HashMap<JobId, AgentId>,
    /// Recent ends nobody was owed, oldest first, at most [`ENDS_KEPT`].
    ended: VecDeque<RunAnswer>,
}

/// Ends kept for the answer-then-end race. The race is a few microseconds wide; this is a bound
/// on memory, not a guess about timing.
const ENDS_KEPT: usize = 16;

impl Late {
    fn engage(&mut self, agent: &AgentId, job: &JobId) {
        // Only the job's own agent: a call naming another agent's job is refused anyway, and must
        // not cost that agent its note on the way.
        if self.owed.get(job) == Some(agent) {
            self.owed.remove(job);
        }
    }

    /// Returns the end to report now, when the job ended between being read as running and here.
    fn told(&mut self, answer: &RunAnswer) -> Option<RunAnswer> {
        let ended = self.ended.iter().position(|end| end.job == answer.job);
        if !answer.running() {
            // The agent has its end; nothing is owed.
            self.owed.remove(&answer.job);
            if let Some(at) = ended {
                self.ended.remove(at);
            }
            return None;
        }
        match ended {
            Some(at) => self.ended.remove(at),
            None => {
                self.owed.insert(answer.job.clone(), answer.agent.clone());
                None
            }
        }
    }

    /// Returns the end back when its agent is owed it.
    fn finished(&mut self, answer: &RunAnswer) -> Option<RunAnswer> {
        if self.owed.remove(&answer.job).is_some() {
            return Some(answer.clone());
        }
        self.ended.push_back(answer.clone());
        while self.ended.len() > ENDS_KEPT {
            self.ended.pop_front();
        }
        None
    }
}

/// How much of a command a note repeats, and how much of its output.
const NOTE_COMMAND_CHARS: usize = 120;
const NOTE_TAIL_LINES: usize = 5;
const NOTE_TAIL_BYTES: usize = 400;

/// The note an agent reads at the start of its next turn: which command, how it ended, where, and
/// the last few lines — enough to go on, with the job id for the rest.
fn late_note(answer: &RunAnswer) -> String {
    let command = clip_chars(answer.command.trim(), NOTE_COMMAND_CHARS);
    let how = match answer.state {
        JobState::Exited { code } => format!("exit code {code}"),
        JobState::Signalled { .. } => format!(
            "ended by {}",
            answer.to_json()["signal_name"].as_str().unwrap_or("a signal")
        ),
        JobState::Running { .. } => "still running".to_string(),
    };
    let stopped = if answer.killed { " (it was stopped)" } else { "" };
    let dir = answer.cwd_after.as_ref().unwrap_or(&answer.cwd);
    let lines: Vec<&str> = answer.tail.lines().collect();
    let last = lines[lines.len().saturating_sub(NOTE_TAIL_LINES)..].join("\n");
    let mut last = last.trim_end().to_string();
    if last.len() > NOTE_TAIL_BYTES {
        let mut from = last.len() - NOTE_TAIL_BYTES;
        while !last.is_char_boundary(from) {
            from += 1;
        }
        last = format!("…{}", &last[from..]);
    }
    let output = if last.is_empty() {
        "It printed nothing.".to_string()
    } else {
        format!("Its last lines:\n{last}")
    };
    format!(
        "Your command `{command}` (job {job}) finished after the call that started it had \
         returned: {how}{stopped}, after {elapsed}, in {dir}. {output}",
        job = answer.job,
        elapsed = human_duration(answer.elapsed),
        dir = dir.display(),
    )
}

fn clip_chars(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_string();
    }
    format!("{}…", text.chars().take(most).collect::<String>())
}

fn human_duration(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    match secs {
        0 => format!("{:.1} s", elapsed.as_secs_f64()),
        1..=59 => format!("{secs} s"),
        60..=3599 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

fn resolver() -> Arc<dyn AgentResolver> {
    RESOLVER
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| Arc::new(NoAgents))
}

/// Kill every agent command, as the shell closes.
pub fn shutdown() {
    if let Some(jobs) = JOBS.get() {
        jobs.shutdown();
    }
}

/// What the dispatch established about one call, read on the UI thread where it is set and
/// carried to the work.
struct Call {
    /// The socket peer, as the kernel reported it — the only account of the caller the token is
    /// checked against.
    pid: Option<u32>,
    /// What rode beside `args`.
    token: Option<String>,
}

impl Call {
    fn current() -> Call {
        Call {
            pid: control::caller().and_then(|c| u32::try_from(c.pid).ok()).filter(|pid| *pid > 0),
            token: control::agent_token(),
        }
    }

    fn agent(&self) -> Result<AgentId, String> {
        resolver().resolve(self.token.as_deref().unwrap_or_default(), self.pid)
    }
}

/// Who the call being dispatched is for, as far as the shell can establish it:
///
/// - `None` — no agent token came with it: the person's own `yos act`, or a caller that runs as
///   no agent. Such a caller is treated as it always was.
/// - `Some(Ok(agent))` — the token names a live agent and the caller descends from the harness it
///   was issued to.
/// - `Some(Err(why))` — a token came and was not believed. Never read as "no agent": a caller that
///   presented a token is not the person.
///
/// Read inside a dispatch, on the handler's thread, where the token and the caller are set — the
/// same place [`Call::current`] reads them — and nowhere else. The token itself goes no further.
pub fn calling_agent() -> Option<Result<AgentId, String>> {
    let call = Call::current();
    call.token.as_ref()?;
    Some(call.agent())
}

/// As [`calling_agent`], but the token is checked against `pid` rather than the socket peer
/// (#182): an app that spends a grant forwards the pid the kernel stamped on the call its token
/// arrived in, because the peer of the forwarded call is the app itself and a token is believed
/// only from the process tree it was issued into. Read in the same place, inside a dispatch.
pub fn calling_agent_at(pid: Option<u32>) -> Option<Result<AgentId, String>> {
    let call = Call::current();
    call.token.as_ref()?;
    Some(resolver().resolve(call.token.as_deref().unwrap_or_default(), pid))
}

/// What `token` names when it is checked against `pid`, outside any dispatch: the shell's own
/// dispatch spends grants in-process, where the token and the kernel's pid for the call arrive
/// on the spender's own arguments instead of in thread-locals (#182). Same resolver, same rule.
pub fn agent_for(token: &str, pid: Option<u32>) -> Result<AgentId, String> {
    resolver().resolve(token, pid)
}

/// The resolver is one global for the whole shell, and tests install tables into it: a test
/// that drives token checks holds this for its length, so two tests' tables cannot replace
/// each other mid-run.
#[cfg(test)]
pub(crate) static RESOLVER_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Hand the work to the socket's side; run it here only when called without a socket.
fn later(work: impl FnOnce() -> Result<Value, String> + Send + 'static) -> Result<Value, String> {
    control::answer_later(work)
        .map(|()| json!({ "answering": "off the UI thread" }))
        .or_else(|work| work())
}

fn text(args: &Value, name: &str) -> String {
    args.get(name).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn job_arg(args: &Value) -> Result<JobId, String> {
    let job = text(args, "job").trim().to_string();
    if job.is_empty() {
        return Err("`job` is empty: the id `agent_run` answered with.".to_string());
    }
    Ok(JobId(job))
}

/// `wait`, in seconds: the design's default, and never past the terminal's own bound.
///
/// A yes or no is read as what it plainly means: `true` waits the default, `false` only starts
/// the command. Minds send `wait: true` whatever the parameter says. On the live instance the
/// Mind did it twice in one turn, was refused twice, and went on without the output it had asked
/// for (2 Oct 2026). Refusing a request whose meaning is not in doubt helps nobody.
fn wait_arg(args: &Value) -> Result<Duration, String> {
    let most = jobs().limits().max_wait.as_secs();
    let Some(given) = args.get("wait").filter(|v| !v.is_null()) else {
        return Ok(DEFAULT_WAIT);
    };
    match given.as_bool() {
        Some(true) => return Ok(DEFAULT_WAIT),
        Some(false) => return Ok(Duration::ZERO),
        None => {}
    }
    let secs = given
        .as_f64()
        .or_else(|| given.as_str().and_then(|s| s.trim().parse().ok()))
        .ok_or_else(|| "`wait` is a number of seconds.".to_string())?;
    if !(0.0..=most as f64).contains(&secs) {
        return Err(format!(
            "`wait` is between 0 and {most} seconds. A command still running when it runs out \
             answers `running: true` with its job id, and `agent_job` waits on it again."
        ));
    }
    Ok(Duration::from_secs_f64(secs))
}

/// The terminal's answer, with the next step spelled out where there is one.
fn answer_json(answer: &RunAnswer) -> Value {
    let mut out = answer.to_json();
    match answer.state {
        JobState::Running { waiting_for_input: true } => {
            out["next"] = "it looks like it is waiting for input: the person can answer in its \
                           card, `agent_input` sends text (end it with \\n to press Return), \
                           `agent_kill` stops it"
                .into();
        }
        JobState::Running { .. } => {
            out["next"] = "still running: `agent_job` waits for it again, `agent_kill` stops it".into();
        }
        _ => {}
    }
    out
}

/// Said once, in every description, because it is the only documentation a mind reads.
const WHO: &str = " Acts for the agent named by the agent token your call carries beside `args` \
                   (`yos act --agent-token`, or YANTRIK_AGENT_TOKEN in yos's environment) — never \
                   an argument.";

/// The four actions as published: what `describe shell` lists and a caller is asked for.
pub(crate) fn specs() -> [Action; 4] {
    [
        // Sensitive, like the Terminal's own `run`: whatever the command does, it does as the
        // person. Deferred because the answer may be `running: true` — the work outlives the call
        // — and the caller has to read `running` rather than assume. Open-ended: the command is
        // whatever it is given, so it asks once in ask, auto and bypass and a session rule
        // answers the rest (29 September 2026).
        Action::new(
            "agent_run",
            &format!(
                "Run one command line in a fresh terminal of your own, in your pane — not the \
                 person's Terminal. Its output is also shown live in a terminal in Mind View, \
                 your desk; `shown_in` in the answer says whether it was. Answers when it exits, with `exit_code` (or `signal`), \
                 `cwd_after` and the `tail` of its output; if it is still going after `wait` it \
                 answers `running: true` with a `job` id. The directory carries to your next \
                 command; exported variables and other shell state do not. The environment is \
                 HOME, USER, PATH, LANG and TERM only.{WHO}"
            ),
        )
        .risk("sensitive")
        .defers()
        .open_ended()
        .arg(Param::text("command").describe(
            "One command line, as it would be typed. Pipes, redirection, `&&` and `cd` work; it \
             runs under bash",
        ))
        .arg(
            Param::text("cwd")
                .optional()
                .describe("Where to run it, absolute or relative to your current directory. Left out: where your last command ended"),
        )
        .arg(
            Param::number("wait")
                .optional()
                .describe("Seconds to wait for it to finish before answering `running: true`. Default 120, at most 600"),
        ),
        Action::new(
            "agent_job",
            &format!(
                "Wait for one of your commands that answered `running: true`, and say where it \
                 stands: the same answer `agent_run` gives. `wait: 0` only looks. Answers early \
                 if the command starts waiting for input.{WHO}"
            ),
        )
        .arg(Param::text("job").describe("The `job` id `agent_run` answered with"))
        .arg(
            Param::number("wait")
                .optional()
                .describe("Seconds to wait for it to finish. Default 120, at most 600"),
        ),
        // Sensitive for the Terminal `send_input`'s reason: a program at a prompt cannot tell
        // these bytes from typing, and the prompt may be `sudo`'s. Open-ended for the same
        // reason: typed into a shell, they are a command.
        Action::new(
            "agent_input",
            &format!(
                "Type into one of your running commands, exactly the characters given — no \
                 newline is added, so end with \\n to press Return; \\u0003 is Ctrl-C. Answers \
                 with where the command stands a moment later.{WHO}"
            ),
        )
        .risk("sensitive")
        .open_ended()
        .arg(Param::text("job").describe("The `job` id `agent_run` answered with"))
        .arg(Param::text("text").describe("The exact characters to send, up to 64 KiB")),
        Action::new(
            "agent_kill",
            &format!(
                "Stop one of your commands: its whole process group gets SIGTERM, and SIGKILL two \
                 seconds later if anything is left. Answers once it has ended, with how.{WHO}"
            ),
        )
        .arg(Param::text("job").describe("The `job` id `agent_run` answered with")),
    ]
}

/// The four actions, for the shell's surface. Each reads the call on the UI thread and hands
/// everything else to the function of the same name.
pub fn actions(surface: ControlSurface) -> ControlSurface {
    let [run, job, input, kill] = specs();
    surface
        .action(run, |args| agent_run(args, Call::current()))
        .action(job, |args| agent_job(args, Call::current()))
        .action(input, |args| agent_input(args, Call::current()))
        .action(kill, |args| agent_kill(args, Call::current()))
}

fn agent_run(args: &Value, call: Call) -> Result<Value, String> {
    let command = text(args, "command");
    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(PathBuf::from);
    let wait = wait_arg(args)?;
    later(move || {
        let agent = call.agent()?;
        // Started, then carded, then waited on: the card opens in the agent's pane the moment the
        // command exists, and fills as its terminal does. `run` would be the same two steps with
        // nothing between them.
        // The command's line is put in the Mind View log first, so no output can arrive ahead of it.
        let mirror = crate::mind_agent_terminal::begin(&agent.0, &command);
        let job = match jobs().start(&agent, &command, cwd) {
            Ok(job) => job,
            Err(e) => {
                if matches!(mirror, crate::mind_agent_terminal::Begin::Log(_)) {
                    crate::mind_agent_terminal::abandon(&agent.0);
                }
                return Err(e);
            }
        };
        let started_in = jobs().answer(&job).map(|a| a.cwd.display().to_string()).unwrap_or_default();
        crate::agents::store().command_started(&agent, &job.0, &command, &started_in);
        // Opened on a worker beside the running command, so neither the first start of Mind View
        // nor the viewer's window wait delays the answer or the command's `wait`. `tail -n +1`
        // shows everything the log already holds.
        let shown = match mirror {
            crate::mind_agent_terminal::Begin::Log(log) => {
                Some(crate::mind_agent_terminal::show_later(&agent.0, log))
            }
            crate::mind_agent_terminal::Begin::Failed(why) => {
                Some(crate::mind_agent_terminal::Shown::Not(why))
            }
            crate::mind_agent_terminal::Begin::Off => None,
        };
        let answer = jobs().job(&agent, &job, wait)?;
        told(&answer);
        let mut out = answer_json(&answer);
        match &shown {
            Some(shown) => {
                if let (Some(into), Some(from)) =
                    (out.as_object_mut(), crate::mind_agent_terminal::answer_fields(shown).as_object())
                {
                    into.extend(from.clone());
                }
            }
            None => {
                out["shown_in"] = "nowhere on screen".into();
                out["shown_note"] = "minds' commands are shown in Mind View only while the setting \
                    `minds open apps in Mind View` is on; the output is in this answer and on the Agents screen"
                    .into();
            }
        }
        Ok(out)
    })
}

fn agent_job(args: &Value, call: Call) -> Result<Value, String> {
    let job = job_arg(args)?;
    let wait = wait_arg(args)?;
    later(move || {
        let agent = call.agent()?;
        engage(&agent, &job);
        let answer = jobs().job(&agent, &job, wait)?;
        told(&answer);
        Ok(answer_json(&answer))
    })
}

fn agent_input(args: &Value, call: Call) -> Result<Value, String> {
    let job = job_arg(args)?;
    let typed = text(args, "text");
    if typed.is_empty() {
        return Err("`text` is empty: the exact characters to send.".to_string());
    }
    later(move || {
        let agent = call.agent()?;
        engage(&agent, &job);
        jobs().input(&agent, &job, &typed)?;
        // A moment for the command to react, so the tail shows what it did with it.
        let answer = jobs().job(&agent, &job, Duration::from_millis(400))?;
        told(&answer);
        let mut out = answer_json(&answer);
        out["sent_bytes"] = typed.len().into();
        Ok(out)
    })
}

fn agent_kill(args: &Value, call: Call) -> Result<Value, String> {
    let job = job_arg(args)?;
    later(move || {
        let agent = call.agent()?;
        // The agent asked for this end, and this call's answer carries it.
        engage(&agent, &job);
        let stopped = jobs().kill(&agent, &job)?;
        let settle = jobs().limits().kill_grace + Duration::from_millis(500);
        let answer = jobs().job(&agent, &job, settle)?;
        told(&answer);
        let mut out = answer_json(&answer);
        out["stopped"] = stopped.into();
        if !stopped {
            out["note"] = "it had already ended; nothing was signalled".into();
        }
        Ok(out)
    })
}

/// How much of a command line `describe` repeats.
const COMMAND_CLIP: usize = 160;

/// Every command running now, oldest first, with the agent it runs for: its job, its command
/// line (cut), how long, and whether it seems to be waiting for input.
pub fn running_jobs() -> Vec<(AgentId, Value)> {
    // Nothing has ever run: say so without starting the store for it.
    let Some(jobs) = JOBS.get() else { return Vec::new() };
    jobs.list()
        .into_iter()
        .filter_map(|summary| {
            let JobState::Running { waiting_for_input } = summary.state else { return None };
            let command: String = summary.command.chars().take(COMMAND_CLIP).collect();
            let entry = json!({
                "job": summary.job,
                "command": if summary.command.chars().count() > COMMAND_CLIP { format!("{command}…") } else { command },
                "elapsed_secs": summary.elapsed.as_secs(),
                "waiting_for_input": waiting_for_input,
            });
            Some((summary.agent, entry))
        })
        .collect()
}

/// `describe shell` → `agent_jobs`: per agent, the commands running now.
pub fn for_describe() -> Value {
    let mut by_agent: Vec<(String, Vec<Value>)> = Vec::new();
    for (agent, entry) in running_jobs() {
        let agent = agent.to_string();
        match by_agent.iter_mut().find(|(a, _)| *a == agent) {
            Some((_, list)) => list.push(entry),
            None => by_agent.push((agent, vec![entry])),
        }
    }
    Value::Array(
        by_agent
            .into_iter()
            .map(|(agent, running)| json!({ "agent": agent, "running": running }))
            .collect(),
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use yantrik_agent_terminal::{TokenTable, NO_AGENT};

    fn call(token: &str) -> Call {
        Call { pid: Some(std::process::id()), token: Some(token.to_string()) }
    }

    /// A call on a session from this process, which stands in for the harness that attached it:
    /// a session answers only the process that attached it and the ones it started.
    fn as_harness(
        host: &yantrik_harness::Host,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        host.handle_from(method, params, Some(std::process::id()), None)
    }

    /// One job's card in its agent's pane, once the store has its end: `(args, provenance, exit
    /// code, output)`. Asked again until the end is there — the terminal answers a waiting call
    /// before its finish listeners have run.
    fn settled_card(agent: &AgentId, job: &str) -> (Value, crate::agents::Provenance, Option<i32>, String) {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            let found = crate::agents::store().read(|s| {
                s.agent(agent)?
                    .cards()
                    .find(|c| c.call == job && !c.running())
                    .map(|c| (c.args.clone(), c.provenance, c.exit_code, c.output.all()))
            });
            if let Some(found) = found {
                return found;
            }
            assert!(std::time::Instant::now() < deadline, "no settled card for {job} in {agent}'s pane");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Each command's terminal lands in the card of the agent it runs for, and in no other — two
    /// agents' commands running side by side, one output each.
    #[test]
    fn a_commands_output_reaches_its_own_agents_card_and_no_other() {
        let a = AgentId::new("pi", "c-glue-out");
        let b = AgentId::new("deepseek", "c-glue-out");
        let ja = jobs().start(&a, "printf 'from-a-1\\n'; sleep 0.3; printf 'from-a-2\\n'", None).unwrap();
        let jb = jobs().start(&b, "printf 'from-b\\n'; sleep 0.1; exit 3", None).unwrap();
        jobs().job(&a, &ja, Duration::from_secs(10)).unwrap();
        jobs().job(&b, &jb, Duration::from_secs(10)).unwrap();

        let (_, provenance, exit, output) = settled_card(&a, &ja.0);
        assert_eq!((provenance, exit), (crate::agents::Provenance::Verified, Some(0)));
        assert!(output.contains("from-a-1") && output.contains("from-a-2"), "{output:?}");
        assert!(!output.contains("from-b"), "another agent's output is not in this card: {output:?}");

        let (_, _, exit, output) = settled_card(&b, &jb.0);
        assert_eq!(exit, Some(3), "the process's own exit code");
        assert!(output.contains("from-b") && !output.contains("from-a"), "{output:?}");

        // Neither job's card is in the other agent's pane at all.
        crate::agents::store().read(|s| {
            assert!(s.agent(&a).unwrap().cards().all(|c| c.call != jb.0));
            assert!(s.agent(&b).unwrap().cards().all(|c| c.call != ja.0));
        });
    }

    /// The token is not an argument of any of the four, so nothing that shows or keeps
    /// arguments — `describe shell`'s action list, an approval card, an audit line — is ever
    /// handed one by these actions.
    #[test]
    fn no_agent_action_takes_the_token_as_an_argument() {
        for spec in specs() {
            let schema = spec.schema();
            let params = &schema["parameters"]["properties"];
            assert!(params.get("agent_token").is_none(), "{} takes the token as an argument: {schema}", spec.name);
            assert!(params.get("agent").is_none(), "{} takes the agent as an argument: {schema}", spec.name);
            assert!(
                schema["description"].as_str().is_some_and(|d| d.contains("beside `args`")),
                "{} does not say where the token goes: {schema}",
                spec.name
            );
        }
    }

    /// The four actions end to end, minus only the socket: with no dispatch in progress the work
    /// runs inline, so this drives the real token check, the real terminal and `describe`.
    ///
    /// One test, because the resolver is the shell's one global: "no tokens yet" has to be seen
    /// before any are installed.
    #[test]
    fn the_actions_answer_for_the_agent_the_token_names_and_nobody_else() {
        // The resolver is the shell's one global; hold it for the length of the test (see
        // `RESOLVER_TESTS`).
        let _held = RESOLVER_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let me = Some(std::process::id());

        // Before the host issues tokens, every call is inert — and says so.
        let err = agent_run(&json!({"command": "echo hi"}), call("t-pi")).unwrap_err();
        assert!(err.starts_with(NO_AGENT), "{err}");
        let err = agent_run(&json!({"command": "echo hi"}), Call { pid: me, token: None }).unwrap_err();
        assert!(err.contains("no agent token came with this call"), "{err}");

        // This test process stands in for the harness that holds both tokens.
        let table = Arc::new(TokenTable::new());
        table.issue("t-pi", AgentId::new("pi", "c-shell"), me);
        table.issue("t-ds", AgentId::new("deepseek", "c-shell"), me);
        install_resolver(table);

        let done = agent_run(&json!({"command": "cd /tmp && echo hi", "wait": 10}), call("t-pi")).unwrap();
        assert_eq!(done["exit_code"], 0, "{done}");
        assert_eq!(done["cwd_after"], "/tmp", "{done}");
        assert_eq!(done["tail"], "hi", "{done}");
        assert_eq!(done["agent"], "pi:c-shell", "the agent is the token's");
        assert!(!done.to_string().contains("t-pi"), "the answer does not repeat the token: {done}");
        // And it is a verified card in pi's pane: the command and where it started, its output,
        // the process's own exit code — and nothing of the token.
        let pi_agent = AgentId::new("pi", "c-shell");
        let job = done["job"].as_str().unwrap().to_string();
        let card = settled_card(&pi_agent, &job);
        assert_eq!(card.0["command"], "cd /tmp && echo hi", "{card:?}");
        assert!(card.0["cwd"].as_str().is_some_and(|c| !c.is_empty()), "{card:?}");
        assert_eq!((card.1, card.2), (crate::agents::Provenance::Verified, Some(0)));
        assert!(card.3.contains("hi") && !card.3.contains("t-pi"), "{card:?}");

        let slow = agent_run(&json!({"command": "sleep 30", "wait": 0.5}), call("t-pi")).unwrap();
        assert_eq!(slow["running"], true, "{slow}");
        assert!(slow["next"].as_str().is_some_and(|n| n.contains("agent_job")), "{slow}");
        let job = slow["job"].clone();

        // `describe shell` lists it under its agent, and nowhere names the token.
        let described = for_describe();
        let pi = described
            .as_array()
            .and_then(|agents| agents.iter().find(|a| a["agent"] == "pi:c-shell"))
            .unwrap_or_else(|| panic!("pi is not listed: {described}"));
        assert!(pi["running"].as_array().unwrap().iter().any(|j| j["job"] == job && j["command"] == "sleep 30"));
        assert!(!described.to_string().contains("t-pi"), "{described}");

        // Another agent's token cannot touch it; nor can pi's token from outside pi's harness.
        let err = agent_kill(&json!({"job": job}), call("t-ds")).unwrap_err();
        assert!(err.contains("belongs to another agent"), "{err}");
        let outsider = Call { pid: Some(1), token: Some("t-pi".into()) };
        let err = agent_job(&json!({"job": job, "wait": 0}), outsider).unwrap_err();
        assert!(err.contains("not issued to the process"), "{err}");

        let looked = agent_job(&json!({"job": job, "wait": 0}), call("t-pi")).unwrap();
        assert_eq!(looked["running"], true);

        let killed = agent_kill(&json!({"job": job}), call("t-pi")).unwrap();
        assert_eq!((killed["stopped"].clone(), killed["signal_name"].clone()), (json!(true), json!("SIGTERM")), "{killed}");

        let answered = agent_run(&json!({"command": "read -r x; echo \"[$x]\"", "wait": 0.3}), call("t-pi")).unwrap();
        let typed = agent_input(&json!({"job": answered["job"], "text": "yes\n"}), call("t-pi")).unwrap();
        assert_eq!(typed["sent_bytes"], 4);
        let finished = agent_job(&json!({"job": answered["job"], "wait": 10}), call("t-pi")).unwrap();
        assert_eq!(finished["tail"], "yes\n[yes]", "the echo of what was typed, then the answer: {finished}");

        // The bounds on `wait`, refused before anything is started.
        let err = agent_run(&json!({"command": "true", "wait": 601}), call("t-pi")).unwrap_err();
        assert!(err.contains("between 0 and 600"), "{err}");
        // A yes or no is what it plainly means, not a refusal: on 561 the Mind sent
        // `wait: true` twice and never saw its output.
        assert_eq!(wait_arg(&json!({"wait": true})).unwrap(), DEFAULT_WAIT);
        assert_eq!(wait_arg(&json!({"wait": false})).unwrap(), Duration::ZERO);
        let yes = agent_run(&json!({"command": "echo yes-wait", "wait": true}), call("t-pi")).unwrap();
        assert_eq!(yes["exit_code"], 0, "`wait: true` waits for the command: {yes}");
        assert!(wait_arg(&json!({"wait": "soon"})).unwrap_err().contains("number of seconds"));

        // ── Wired to the harness host, as the shell does when it makes one ──
        //
        // This test process stands in for the harness again: it attaches with its own pid, the
        // way the socket's SO_PEERCRED would report it, and its agent is handed a turn and so a
        // token.
        use yantrik_harness::{protocol, Host, Turn};
        let host = Host::new(vec![]);
        let attach = json!({ "id": "pi", "name": "Pi", "conversations": true });
        let session = host.handle_from(protocol::ATTACH, &attach, me, None).unwrap()["session"].as_str().unwrap().to_string();
        let agent = host.start_agent("pi").unwrap();
        let turn = |text: &str| {
            let _answer = host.send_to(&agent, Turn::new(text)).unwrap();
            let handed = as_harness(&host, protocol::POLL, &json!({ "session": session })).unwrap();
            as_harness(&host, protocol::COMPLETE, &json!({ "session": session, "turn_id": handed["turn_id"] })).unwrap();
            handed
        };
        let token = turn("build it")["agent_token"].as_str().unwrap().to_string();
        serve_host(&host);

        // The table's tokens name nothing now; the host's do, for its agent.
        assert!(agent_run(&json!({"command": "true"}), call("t-pi")).unwrap_err().starts_with(NO_AGENT));
        let slow = agent_run(&json!({"command": "sleep 1; echo built", "wait": 0.2}), call(&token)).unwrap();
        assert_eq!((slow["running"].clone(), slow["agent"].clone()), (json!(true), json!(agent.to_string())), "{slow}");
        // A command whose end is carried by the agent's own later call is not owed a note.
        let waited = agent_run(&json!({"command": "sleep 1.5; echo waited", "wait": 0.1}), call(&token)).unwrap();
        let ended = agent_job(&json!({"job": waited["job"], "wait": 10}), call(&token)).unwrap();
        assert_eq!(ended["exit_code"], 0, "{ended}");

        // The first one finished after its call returned: its end rides into the agent's next
        // turn, once, as a note in the context. Asked again until it is there, not after a sleep
        // that a loaded machine could outlast.
        let notes_in = |handed: &Value| -> Vec<String> {
            let context: Value = handed["context"].as_str().map(|c| serde_json::from_str(c).unwrap()).unwrap_or_default();
            context["notes"].as_array().into_iter().flatten().map(|n| n.as_str().unwrap().to_string()).collect()
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let notes = loop {
            let notes = notes_in(&turn("what happened?"));
            if !notes.is_empty() || std::time::Instant::now() > deadline {
                break notes;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(notes.len(), 1, "one late end, one note: {notes:?}");
        let note = &notes[0];
        for said in ["`sleep 1; echo built`", slow["job"].as_str().unwrap(), "exit code 0", "built"] {
            assert!(note.contains(said), "the note does not say {said:?}: {note}");
        }
        assert!(!note.contains(&token), "the note does not carry the token");
        assert!(notes_in(&turn("and now?")).is_empty(), "a note is delivered once");
    }
}

/// What does not need a terminal: which ends are owed, the note's words, and the wiring.
#[cfg(test)]
mod late_tests {
    use super::*;
    use std::path::PathBuf;

    fn answer(job: &str, running: bool) -> RunAnswer {
        RunAnswer {
            job: JobId(job.to_string()),
            agent: AgentId::new("pi", "c-7f3a91"),
            command: "cargo build --release".to_string(),
            cwd: PathBuf::from("/home/me/proj"),
            cwd_after: (!running).then(|| PathBuf::from("/home/me/proj/target")),
            state: if running { JobState::Running { waiting_for_input: false } } else { JobState::Exited { code: 101 } },
            killed: false,
            elapsed: Duration::from_secs(134),
            tail: "Compiling a\nCompiling b\nerror[E0425]: cannot find value `x`\nerror: could not compile".to_string(),
            tail_clipped: false,
            output_bytes: 900,
            truncated_bytes: 0,
        }
    }

    #[test]
    fn an_end_is_owed_only_when_the_agents_last_word_on_it_was_running() {
        let mut late = Late::default();
        // Told running, then it ends: owed.
        assert_eq!(late.told(&answer("j1", true)), None);
        assert_eq!(late.finished(&answer("j1", false)), Some(answer("j1", false)));
        assert!(late.owed.is_empty() && late.ended.is_empty(), "owed once, and settled");

        // Ended within the call that started it: the answer carried it, nothing is owed.
        assert_eq!(late.finished(&answer("j2", false)), None);
        assert_eq!(late.told(&answer("j2", false)), None);
        assert!(late.ended.is_empty() && late.owed.is_empty());
    }

    #[test]
    fn an_agent_waiting_on_its_command_again_is_answered_by_that_call_not_by_a_note() {
        let mut late = Late::default();
        let pi = AgentId::new("pi", "c-7f3a91");
        late.told(&answer("j1", true));
        // `agent_job` begins, and the command ends while it waits: its answer carries the end.
        late.engage(&pi, &JobId("j1".into()));
        assert_eq!(late.finished(&answer("j1", false)), None);
        assert_eq!(late.told(&answer("j1", false)), None);
        // …but when that call too runs out first, the end is owed again.
        late.told(&answer("j3", true));
        late.engage(&pi, &JobId("j3".into()));
        late.told(&answer("j3", true));
        assert!(late.finished(&answer("j3", false)).is_some());
        // Another agent naming this job does not take its note away.
        late.told(&answer("j4", true));
        late.engage(&AgentId::new("deepseek", "c-02be44"), &JobId("j4".into()));
        assert!(late.finished(&answer("j4", false)).is_some());
    }

    #[test]
    fn an_end_between_the_look_and_the_record_is_still_reported() {
        // The answer was read as running, and the job ended before that answer reached here.
        let mut late = Late::default();
        assert_eq!(late.finished(&answer("j1", false)), None);
        assert_eq!(late.told(&answer("j1", true)), Some(answer("j1", false)));
        assert!(late.ended.is_empty() && late.owed.is_empty());
        // And the record of unclaimed ends is bounded.
        for n in 0..ENDS_KEPT + 10 {
            late.finished(&answer(&format!("x{n}"), false));
        }
        assert_eq!(late.ended.len(), ENDS_KEPT);
    }

    #[test]
    fn the_note_says_which_command_how_it_ended_where_and_its_last_lines() {
        let note = late_note(&answer("job-1a2b", false));
        for said in ["`cargo build --release`", "job-1a2b", "exit code 101", "after 2m 14s",
                     "in /home/me/proj/target", "error: could not compile"] {
            assert!(note.contains(said), "{said:?} missing: {note}");
        }
        assert!(note.len() < yantrik_harness::host::MAX_NOTE_BYTES, "{} bytes", note.len());

        let mut killed = answer("job-9", false);
        killed.state = JobState::Signalled { signal: 15 };
        killed.killed = true;
        killed.tail = String::new();
        killed.command = "x".repeat(500);
        let note = late_note(&killed);
        assert!(note.contains("ended by SIGTERM (it was stopped)") && note.contains("It printed nothing."), "{note}");
        assert!(note.contains(&format!("`{}…`", "x".repeat(NOTE_COMMAND_CHARS))), "the command is cut: {note}");

        let mut chatty = answer("job-8", false);
        chatty.tail = "é".repeat(2000);
        assert!(late_note(&chatty).len() < yantrik_harness::host::MAX_NOTE_BYTES);
    }

    /// The shell has one place where the host is made, and that is where the agent terminal is
    /// tied to it. A missing call compiles, and every agent's command is then answered "no agent
    /// holds this token" — so the call is asserted where it lives.
    #[test]
    fn the_shell_ties_the_agent_terminal_to_the_host_when_it_makes_it() {
        let wiring = include_str!("wire/harness.rs");
        let made = wiring.find("Host::new(").expect("wire/harness.rs makes the host");
        let tied = wiring.find("control_agent_terminal::serve_host(&host)").expect(
            "wire/harness.rs does not call control_agent_terminal::serve_host(&host): agent tokens would never resolve",
        );
        assert!(tied > made, "serve_host is called before the host exists");
        let this = include_str!("control_agent_terminal.rs");
        let body = &this[this.find("pub fn serve_host(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("HostTokens::new(host.clone())") && body.contains("host.note_for(agent, note)"), "{body}");
    }
}
