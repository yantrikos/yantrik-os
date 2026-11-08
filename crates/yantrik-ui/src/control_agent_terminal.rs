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

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use serde_json::{json, Value};
use yantrik_agent_terminal::{
    AgentId, AgentResolver, HostTokens, JobId, JobState, Jobs, Limits, NoAgents, RunAnswer, Sealed,
    DEFAULT_WAIT,
};
use yantrik_app_runtime::control::{self, Action, App as ControlSurface, Param};

use crate::control_agent_late::{engage, late, report_late, report_late_finishes, told};

static JOBS: OnceLock<Jobs> = OnceLock::new();
static RESOLVER: RwLock<Option<Arc<dyn AgentResolver>>> = RwLock::new(None);

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
pub(crate) struct Call {
    /// The socket peer, as the kernel reported it — the only account of the caller the token is
    /// checked against.
    pid: Option<u32>,
    /// What rode beside `args`.
    token: Option<String>,
}

impl Call {
    pub(crate) fn current() -> Call {
        Call {
            pid: control::caller().and_then(|c| u32::try_from(c.pid).ok()).filter(|pid| *pid > 0),
            token: control::agent_token(),
        }
    }

    pub(crate) fn agent(&self) -> Result<AgentId, String> {
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

pub(crate) fn text(args: &Value, name: &str) -> String {
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
pub(crate) fn wait_arg(args: &Value) -> Result<Duration, String> {
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
                 `cwd_after` and the `tail` of its output; if it is still going after `wait` seconds \
                 it answers `running: true` with a `job` id. The directory carries to your next \
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
                .describe(
                    "A number of seconds, e.g. 30, not true or false. How long to wait for it to finish \
                     before answering `running: true`. Default 120, at most 600",
                ),
        ),
        Action::new(
            "agent_job",
            &format!(
                "Wait for one of your commands that answered `running: true`, and say where it \
                 stands: the same answer `agent_run` gives. `wait` is a number of \
                 seconds, and `wait: 0` only looks. Answers early \
                 if the command starts waiting for input.{WHO}"
            ),
        )
        .arg(Param::text("job").describe("The `job` id `agent_run` answered with"))
        .arg(
            Param::number("wait")
                .optional()
                .describe(
                    "A number of seconds, e.g. 30, not true or false. How long to wait for it to finish. \
                     Default 120, at most 600",
                ),
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
    let named = command.clone();
    run_job(
        command,
        wait,
        call,
        Box::new(move |agent| {
            // An unsealed command does not start in a workshop or name one: what a Mind wrote there
            // is not run outside the sandbox (#1124, H2). Where it would start is where it asked,
            // or where its last command ended.
            let here = jobs().directory(agent);
            let start = match &cwd {
                Some(dir) if dir.is_absolute() => dir.clone(),
                Some(dir) => here.join(dir),
                None => here,
            };
            let record = crate::workshop::record_path();
            crate::workshop_guard::refuse_unsealed(&record, &named, &start)?;
            // A command that may read a tainted workshop's files (it starts in a folder above it,
            // or names one, `grep -r` over the Documents) makes the conversation a tainted one:
            // what is in there was written by a conversation that had read private state (#1124, N4).
            if let Some(why) = crate::workshop_guard::reaches_tainted(&record, &crate::workshop::home(), &named, &start) {
                crate::control_taint_scope::mark_agent_read(agent, &why);
            }
            Ok(Prepared { cwd, sealed: None })
        }),
    )
}

/// Where a command starts and whether it is sealed, settled once the agent is known. Sealed only
/// from `control_workshop`, which has checked the grant itself.
pub(crate) struct Prepared {
    pub cwd: Option<PathBuf>,
    pub sealed: Option<Sealed>,
}

pub(crate) type Prepare = Box<dyn FnOnce(&AgentId) -> Result<Prepared, String> + Send>;

/// One command for the calling agent, in its terminal and in Mind View: the whole of `agent_run`,
/// and of `agent_run_sealed` with a sandbox.
pub(crate) fn run_job(command: String, wait: Duration, call: Call, prepare: Prepare) -> Result<Value, String> {
    later(move || {
        let agent = call.agent()?;
        let Prepared { cwd, sealed } = prepare(&agent)?;
        // Started, then carded, then waited on: the card opens in the agent's pane the moment the
        // command exists, and fills as its terminal does. `run` would be the same two steps with
        // nothing between them.
        // The command's line is put in the Mind View log first, so no output can arrive ahead of it.
        let mirror = crate::mind_agent_terminal::begin(&agent.0, &command);
        let job = match jobs().start_in(&agent, &command, cwd, sealed.as_ref()) {
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
        if sealed.is_some() {
            out["sealed"] = true.into();
        }
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
#[path = "control_agent_terminal_tests.rs"]
mod tests;
