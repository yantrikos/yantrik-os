//! Commands that finish after their call returned, and the note an agent reads next turn.
//!
//! Split from `control_agent_terminal`. `agent_run` answers `running: true` when its `wait` runs
//! out; when that command ends, nobody's call carries the end, so the agent is told in its next
//! turn instead: the desktop leaves it a note (`Host::note_for`). Which ends are owed is decided
//! from what each agent was last *told*: a job whose last answer said `running` is owed its end,
//! unless the agent is waiting on it again (`agent_job`, `agent_input`, `agent_kill`), in which
//! case that call's answer carries it. See [`Late`].

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use yantrik_agent_terminal::{AgentId, JobId, JobState, RunAnswer};

/// Where a late finish is reported: `(agent, note) → kept?`. The host's `note_for`, once
/// `control_agent_terminal::serve_host` has run; until then a late finish is noticed and not reported.
type Deliver = Arc<dyn Fn(&AgentId, String) -> bool + Send + Sync>;
static DELIVER: RwLock<Option<Deliver>> = RwLock::new(None);

/// Where a command that finished after its call returned is reported. Replaces any earlier one.
pub fn report_late_finishes(deliver: impl Fn(&AgentId, String) -> bool + Send + Sync + 'static) {
    *DELIVER.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(deliver));
}

/// The one record of which ends are owed.
pub(crate) fn late() -> std::sync::MutexGuard<'static, Late> {
    static LATE: OnceLock<Mutex<Late>> = OnceLock::new();
    LATE.get_or_init(|| Mutex::new(Late::default())).lock().unwrap_or_else(|e| e.into_inner())
}

/// An agent's call on `job` has begun; whatever it answers decides afresh whether the end is owed.
pub(crate) fn engage(agent: &AgentId, job: &JobId) {
    late().engage(agent, job);
}

/// The agent has just been given `answer`.
pub(crate) fn told(answer: &RunAnswer) {
    let owed = late().told(answer);
    if let Some(done) = owed {
        report_late(&done);
    }
}

pub(crate) fn report_late(done: &RunAnswer) {
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
pub(crate) struct Late {
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
    pub(crate) fn finished(&mut self, answer: &RunAnswer) -> Option<RunAnswer> {
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
