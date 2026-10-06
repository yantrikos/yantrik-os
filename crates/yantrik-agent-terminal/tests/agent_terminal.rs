//! The agent terminal, driven the way the shell drives it: real bash, real PTYs, real process
//! groups. Linux only — everything here is `/proc`, `setsid` and a pseudo-terminal.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yantrik_agent_terminal::{dropped_marker, AgentId, JobState, Jobs, Limits};

fn pi() -> AgentId {
    AgentId::new("pi", "c-7f3a91")
}

fn deepseek() -> AgentId {
    AgentId::new("deepseek", "c-02be44")
}

/// The design's limits, with the clocks shortened so a test does not sit through twenty seconds of
/// silence or two of grace.
fn quick() -> Limits {
    Limits { silence: Duration::from_millis(1500), kill_grace: Duration::from_millis(500), ..Limits::default() }
}

fn jobs() -> Jobs {
    Jobs::new(quick())
}

const LONG: Duration = Duration::from_secs(20);

/// A pid is gone once `/proc` has no live process for it: absent, or a zombie waiting on init.
fn gone(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(_) => true,
        Ok(stat) => stat[stat.rfind(')').unwrap() + 1..].trim_start().starts_with('Z'),
    }
}

fn eventually(what: &str, ok: impl FnMut() -> bool) {
    within(Duration::from_secs(10), what, ok)
}

fn within(limit: Duration, what: &str, mut ok: impl FnMut() -> bool) {
    let until = Instant::now() + limit;
    while !ok() {
        assert!(Instant::now() < until, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// The number after `label=` in a tail.
fn number_after(tail: &str, label: &str) -> u32 {
    let at = tail.find(label).unwrap_or_else(|| panic!("no `{label}` in: {tail}")) + label.len();
    tail[at..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap()
}

#[test]
fn an_exit_code_and_a_killing_signal_are_both_reported() {
    let jobs = jobs();
    let three = jobs.run(&pi(), "echo about to fail; exit 3", None, LONG).unwrap();
    assert_eq!(three.state, JobState::Exited { code: 3 });
    assert!(three.tail.contains("about to fail"), "{}", three.tail);

    let status = jobs.run(&pi(), "false", None, LONG).unwrap();
    assert_eq!(status.exit_code(), Some(1), "the last command's status is the command's status");

    // Nothing can trap SIGKILL, so the shell really does die of it, and the answer says so.
    let killed = jobs.run(&pi(), "kill -KILL $$", None, LONG).unwrap();
    assert_eq!(killed.state, JobState::Signalled { signal: 9 });
    let json = killed.to_json();
    assert_eq!(json["signal_name"], "SIGKILL");
    assert_eq!(json["running"], false);
    assert!(json.get("exit_code").is_none(), "an exit code or a signal, never both: {json}");
}

#[test]
fn the_directory_carries_to_the_next_command() {
    let jobs = jobs();
    let moved = jobs.run(&pi(), "cd /tmp", None, LONG).unwrap();
    assert_eq!(moved.exit_code(), Some(0));
    assert_eq!(moved.cwd_after.as_deref(), Some(Path::new("/tmp")));

    let next = jobs.run(&pi(), "pwd", None, LONG).unwrap();
    assert_eq!(next.cwd, PathBuf::from("/tmp"), "the next command starts where the last ended");
    assert_eq!(next.tail.trim(), "/tmp");

    // A program after the `cd` — the case where bash could otherwise exec the last command and
    // skip the trap that reports the directory.
    let then_ls = jobs.run(&pi(), "cd / && ls >/dev/null", None, LONG).unwrap();
    assert_eq!(then_ls.cwd_after.as_deref(), Some(Path::new("/")));
    // And `exit` on the way out still reports it.
    let exits = jobs.run(&pi(), "cd /usr; exit 4", None, LONG).unwrap();
    assert_eq!((exits.exit_code(), exits.cwd_after.as_deref()), (Some(4), Some(Path::new("/usr"))));
    assert_eq!(jobs.directory(&pi()), PathBuf::from("/usr"));

    // One agent's directory is its own.
    assert_ne!(jobs.directory(&deepseek()), PathBuf::from("/usr"));
    // An explicit `cwd`, relative to where the agent is.
    let relative = jobs.run(&pi(), "pwd", Some(PathBuf::from("bin")), LONG).unwrap();
    assert_eq!(relative.tail.trim(), "/usr/bin");
    let err = jobs.start(&pi(), "pwd", Some(PathBuf::from("/no/such/dir"))).unwrap_err();
    assert!(err.contains("is not a directory"), "{err}");
}

#[test]
fn an_exported_variable_does_not_carry_to_the_next_command() {
    let jobs = jobs();
    let set = jobs.run(&pi(), "export YANTRIK_CARRY=yes; f() { echo fn; }; echo \"[$YANTRIK_CARRY]\"", None, LONG).unwrap();
    assert_eq!(set.tail.trim(), "[yes]", "inside its own command the variable is there");

    let next = jobs
        .run(&pi(), "printf '[%s]\\n' \"$YANTRIK_CARRY\"; type f >/dev/null 2>&1 || echo no-function", None, LONG)
        .unwrap();
    assert_eq!(next.tail.trim(), "[]\nno-function", "shell state ends with the command");
}

#[test]
fn a_slow_command_answers_running_and_a_later_wait_answers_finished() {
    let jobs = jobs();
    let early = jobs.run(&pi(), "sleep 3; echo slept", None, Duration::from_secs(1)).unwrap();
    assert!(early.running(), "{early:?}");
    assert_eq!(early.state, JobState::Running { waiting_for_input: false });
    assert!(early.elapsed >= Duration::from_millis(900) && early.elapsed < Duration::from_secs(3));
    assert_eq!(early.to_json()["running"], true);

    let later = jobs.job(&pi(), &early.job, LONG).unwrap();
    assert_eq!(later.exit_code(), Some(0));
    assert!(later.tail.contains("slept"), "{}", later.tail);
    assert_eq!(later.job, early.job);

    // Looking again at a finished job is free and says the same thing.
    let again = jobs.job(&pi(), &early.job, Duration::ZERO).unwrap();
    assert_eq!(again.state, later.state);
}

#[test]
fn a_kill_takes_the_whole_process_group_not_one_pid() {
    // A long grace, so the child has to die of the TERM itself: the SIGKILL that follows the grace
    // goes to the group too and would hide a TERM sent to the leader alone.
    let jobs = Jobs::new(Limits { kill_grace: Duration::from_secs(8), ..quick() });
    // A child that survives a hangup, so only a signal to the whole group can reach it.
    let started = jobs
        .run(&pi(), "nohup sleep 300 >/dev/null 2>&1 & echo child=$!; wait", None, Duration::from_millis(800))
        .unwrap();
    assert!(started.running());
    let child = number_after(&started.tail, "child=");
    assert!(!gone(child), "the background child is running before the kill");

    assert_eq!(jobs.kill(&pi(), &started.job), Ok(true));
    let after = jobs.job(&pi(), &started.job, LONG).unwrap();
    assert_eq!(after.state, JobState::Signalled { signal: 15 });
    assert!(after.killed);
    within(Duration::from_secs(3), "the background child to die of the group's TERM", || gone(child));

    // Killing it again is not an error; it says it had already ended.
    assert_eq!(jobs.kill(&pi(), &started.job), Ok(false));
}

#[test]
fn a_command_that_ignores_term_is_killed_after_the_grace() {
    let jobs = jobs();
    let started = jobs.run(&pi(), "trap '' TERM; sleep 300", None, Duration::from_millis(300)).unwrap();
    assert!(started.running());
    let asked = Instant::now();
    jobs.kill(&pi(), &started.job).unwrap();
    let after = jobs.job(&pi(), &started.job, LONG).unwrap();
    assert_eq!(after.state, JobState::Signalled { signal: 9 }, "TERM was ignored, KILL was not");
    assert!(asked.elapsed() >= quick().kill_grace, "the grace period was given first");
}

#[test]
fn another_agents_job_is_refused_with_a_sentence() {
    let jobs = jobs();
    let mine = jobs.run(&pi(), "sleep 5", None, Duration::ZERO).unwrap();

    for refused in [
        jobs.job(&deepseek(), &mine.job, Duration::ZERO).map(|_| ()),
        jobs.input(&deepseek(), &mine.job, "y\n"),
        jobs.kill(&deepseek(), &mine.job).map(|_| ()),
    ] {
        let err = refused.unwrap_err();
        assert!(err.contains("belongs to another agent"), "{err}");
    }
    assert!(jobs.job(&pi(), &mine.job, Duration::ZERO).unwrap().running(), "and nothing happened to it");
    assert!(jobs.kill_agent(&deepseek()).is_empty(), "stopping one agent stops only its own");

    let err = jobs
        .job(&pi(), &yantrik_agent_terminal::JobId("job-0000".into()), Duration::ZERO)
        .unwrap_err();
    assert!(err.contains("no job `job-0000`"), "{err}");
    // Ids are long and unguessable.
    assert_eq!(mine.job.0.len(), "job-".len() + 32, "{}", mine.job);

    assert_eq!(jobs.kill_agent(&pi()), vec![mine.job.clone()]);
}

#[test]
fn the_environment_is_built_and_nothing_of_the_parents_reaches_the_command() {
    // Set in this process, which is the shell's stand-in: a key a harness was started with.
    std::env::set_var("YANTRIK_AGENT_TERMINAL_LEAK", "leaked");
    let jobs = jobs();
    let listed = jobs.run(&pi(), "env", Some(PathBuf::from("/tmp")), LONG).unwrap();
    assert!(!listed.tail.contains("YANTRIK_AGENT_TERMINAL_LEAK"), "{}", listed.tail);

    let names: Vec<&str> = listed.tail.lines().filter_map(|l| l.split_once('=').map(|(k, _)| k)).collect();
    // bash itself exports SHLVL and `_`; everything else is ours.
    let allowed = ["HOME", "USER", "PATH", "LANG", "TERM", "PWD", "SHLVL", "_", "OLDPWD"];
    for name in &names {
        assert!(allowed.contains(name), "`{name}` reached the command: {}", listed.tail);
    }
    for needed in ["HOME", "USER", "PATH", "LANG", "TERM", "PWD"] {
        assert!(names.contains(&needed), "`{needed}` is missing: {}", listed.tail);
    }
    assert!(listed.tail.lines().any(|l| l == "TERM=xterm-256color"));
    assert!(listed.tail.lines().any(|l| l == "PWD=/tmp"));
}

#[test]
fn past_two_mib_the_output_keeps_its_head_and_tail_with_a_marker() {
    let jobs = jobs();
    let three_mib = 3 * 1024 * 1024;
    let done = jobs
        .run(&pi(), &format!("printf start; head -c {three_mib} /dev/zero | tr '\\0' a; printf end"), None, LONG)
        .unwrap();
    assert_eq!(done.exit_code(), Some(0));
    let written = (three_mib + "start".len() + "end".len()) as u64;
    assert_eq!(done.output_bytes, written);
    assert_eq!(done.truncated_bytes, written - 2 * 1024 * 1024);
    assert!(done.tail.ends_with("end"), "the answer's tail is the end of the output");
    assert!(done.tail.len() <= quick().tail_bytes, "and capped: {} bytes", done.tail.len());
    assert!(done.tail_clipped);

    let kept = jobs.output(&done.job).unwrap();
    let marker = dropped_marker(done.truncated_bytes);
    assert_eq!(kept.len(), 2 * 1024 * 1024 + marker.len());
    assert!(kept.starts_with(b"start"), "the head is kept");
    assert!(kept.ends_with(b"end"), "the tail is kept");
    let text = String::from_utf8_lossy(&kept);
    assert!(text.contains(&format!("{} bytes of output dropped", done.truncated_bytes)));
}

#[test]
fn input_reaches_a_command_reading_its_terminal() {
    let jobs = jobs();
    let asking = jobs
        .run(&pi(), "read -r answer; echo \"got:$answer\"", None, Duration::from_millis(500))
        .unwrap();
    assert!(asking.running());
    jobs.input(&pi(), &asking.job, "hello there\n").unwrap();
    let answered = jobs.job(&pi(), &asking.job, LONG).unwrap();
    assert_eq!(answered.exit_code(), Some(0));
    assert!(answered.tail.contains("got:hello there"), "{}", answered.tail);

    let err = jobs.input(&pi(), &asking.job, "more\n").unwrap_err();
    assert!(err.contains("has finished"), "{err}");
}

#[test]
fn a_silent_prompt_is_flagged_as_waiting_for_input_and_the_wait_returns_early() {
    let jobs = jobs();
    let asked = Instant::now();
    let prompt = jobs
        .run(&pi(), "read -r -p 'your name? ' who; echo \"hi $who\"", None, LONG)
        .unwrap();
    assert_eq!(prompt.state, JobState::Running { waiting_for_input: true }, "{prompt:?}");
    assert!(
        asked.elapsed() < Duration::from_secs(8),
        "a wait on a job asking for input returns when it starts asking, not at the end of its budget"
    );
    assert!(prompt.tail.contains("your name?"), "the prompt is in the tail: {}", prompt.tail);
    assert_eq!(prompt.to_json()["waiting_for_input"], true);

    // Silence alone is not waiting: a sleeping command is quiet but reads nothing.
    let quiet = jobs.run(&pi(), "sleep 4", None, Duration::from_millis(3000)).unwrap();
    assert_eq!(quiet.state, JobState::Running { waiting_for_input: false });

    jobs.input(&pi(), &prompt.job, "Pranab\n").unwrap();
    let answered = jobs.job(&pi(), &prompt.job, LONG).unwrap();
    assert!(answered.tail.contains("hi Pranab"), "{}", answered.tail);
}

#[test]
fn one_agent_may_run_four_commands_at_once_and_the_desktop_sixteen() {
    let jobs = Jobs::new(Limits { per_agent: 2, total: 3, ..quick() });
    jobs.start(&pi(), "sleep 5", None).unwrap();
    jobs.start(&pi(), "sleep 5", None).unwrap();
    let err = jobs.start(&pi(), "sleep 5", None).unwrap_err();
    assert!(err.contains("already has 2 commands running"), "{err}");

    jobs.start(&deepseek(), "sleep 5", None).unwrap();
    let err = jobs.start(&AgentId::new("openclaw", "main"), "sleep 5", None).unwrap_err();
    assert!(err.contains("3 commands are running on this desktop"), "{err}");

    // The design's own numbers.
    assert_eq!((Limits::default().per_agent, Limits::default().total), (4, 16));
}

#[test]
fn the_latest_started_command_decides_the_directory_not_the_last_to_finish() {
    let jobs = jobs();
    let slow = jobs.start(&pi(), "cd /; sleep 2", None).unwrap();
    let quick_cd = jobs.run(&pi(), "cd /tmp", None, LONG).unwrap();
    assert_eq!(quick_cd.cwd_after.as_deref(), Some(Path::new("/tmp")));
    let slow = jobs.job(&pi(), &slow, LONG).unwrap();
    assert_eq!(slow.cwd_after.as_deref(), Some(Path::new("/")), "it reports where it ended…");
    assert_eq!(jobs.directory(&pi()), PathBuf::from("/tmp"), "…but a slow old command never moves the agent back");
}

#[test]
fn the_view_gets_the_raw_bytes_the_screen_and_the_end() {
    let jobs = jobs();
    let bytes = Arc::new(Mutex::new(Vec::<u8>::new()));
    let owners = Arc::new(Mutex::new(Vec::<AgentId>::new()));
    let ended = Arc::new(Mutex::new(Vec::new()));
    jobs.on_output({
        let (bytes, owners) = (bytes.clone(), owners.clone());
        move |agent, _job, chunk| {
            owners.lock().unwrap().push(agent.clone());
            bytes.lock().unwrap().extend_from_slice(chunk);
        }
    });
    jobs.on_finish({
        let ended = ended.clone();
        move |answer| ended.lock().unwrap().push((answer.job.clone(), answer.exit_code()))
    });
    // A second listener — the view's and the glue's are both wired — is added, not swapped in.
    let also = Arc::new(Mutex::new(Vec::new()));
    jobs.on_finish({
        let also = also.clone();
        move |answer| also.lock().unwrap().push(answer.job.clone())
    });
    // The same for the bytes: the Agents store's card and any other view each hear all of them.
    let also_bytes = Arc::new(Mutex::new(Vec::<u8>::new()));
    jobs.on_output({
        let also_bytes = also_bytes.clone();
        move |_agent, _job, chunk| also_bytes.lock().unwrap().extend_from_slice(chunk)
    });

    let done = jobs.run(&pi(), "printf '\\033[31mred\\033[0m\\n'", None, LONG).unwrap();
    let raw = bytes.lock().unwrap().clone();
    assert!(raw.windows(5).any(|w| w == b"\x1b[31m"), "the raw bytes keep their escapes");
    assert_eq!(*also_bytes.lock().unwrap(), raw, "a second output listener hears every byte the first does");
    assert!(owners.lock().unwrap().iter().all(|a| a == &pi()), "every chunk names its agent");
    assert_eq!(done.tail, "red", "the answer's tail is what the screen shows, escapes gone");
    let red = jobs
        .with_screen(&done.job, |screen| screen.cell(0, 0).map(|c| c.fgcolor()))
        .flatten();
    assert_eq!(red, Some(yantrik_agent_terminal::vt100::Color::Idx(1)), "the screen keeps the colour");
    eventually("the finish to be reported", || !ended.lock().unwrap().is_empty());
    assert_eq!(ended.lock().unwrap()[0], (done.job.clone(), Some(0)));
    eventually("the second listener to hear it too", || !also.lock().unwrap().is_empty());
    assert_eq!(*also.lock().unwrap(), [done.job.clone()]);
    assert_eq!(jobs.owner(&done.job), Some(pi()));
}

#[test]
fn every_running_group_dies_when_the_jobs_are_dropped() {
    let jobs = Jobs::new(Limits { kill_grace: Duration::from_secs(8), ..quick() });
    let started = jobs
        .run(&pi(), "nohup sleep 300 >/dev/null 2>&1 & echo child=$!; wait", None, Duration::from_millis(800))
        .unwrap();
    let child = number_after(&started.tail, "child=");
    assert!(!gone(child));
    drop(jobs);
    eventually("the shell closing to take the command with it", || gone(child));
}

/// The shell refuses a run stamp from inside a running command (#667 review, round 2): it needs
/// the pid of every command still running, and only those.
#[test]
fn live_pids_are_the_running_commands_and_only_those() {
    let jobs = jobs();
    let job = jobs.start(&pi(), "sleep 30", None).unwrap();
    let live = jobs.live_pids();
    assert_eq!(live.len(), 1, "{live:?}");
    assert!(std::fs::read_to_string(format!("/proc/{}/stat", live[0])).is_ok(), "a real process");
    jobs.kill(&pi(), &job).unwrap();
    eventually("the killed command is no longer live", || jobs.live_pids().is_empty());
    let done = jobs.start(&deepseek(), "true", None).unwrap();
    let _ = jobs.job(&deepseek(), &done, LONG);
    eventually("a finished command is not live", || jobs.live_pids().is_empty());
}
