//! An agent's commands, each in a terminal of its own that the shell owns.
//!
//! # Why
//!
//! Until now a mind that ran a command typed it into the person's own Terminal tab, waited up to
//! 900 ms on the Terminal's UI thread, and got back whatever lines had appeared by then — no exit
//! status, nothing that came later, and two minds (or a mind and the person) sharing one shell.
//! `design/agents-workspace-2026-09-23.md`, decision 3, replaces that: **one command, one PTY, one
//! card**, owned by the shell and drawn in the pane of the agent that ran it. This crate is that
//! terminal, with no Slint and no socket in it, so every rule below is tested without a shell.
//!
//! # The rule about state
//!
//! **The working directory carries from one of an agent's commands to the next; shell state does
//! not.** Exported variables, functions, aliases and activated environments end with the command
//! that made them. That is exactly the rule of Claude Code's own Bash tool ("the working directory
//! persists between commands, shell state does not"), which is the experience this is measured
//! against, and it is what a model's commands actually need: `cd` is the one piece of state a
//! model relies on carrying, and it is the one piece that can be read back honestly.
//!
//! Read back, not guessed. Each command runs as `bash -c` under a small wrapper that `eval`s it in
//! the agent's directory and, on the way out (an `EXIT` trap, so `exit 3` is covered too), writes
//! `$PWD` to a separate pipe on fd 3. The directory the next command starts in is the one this
//! command *ended* in, as the command itself reported it. A command that `exec`s, is killed, or
//! replaces the trap reports nothing, and the agent stays where it was. When an agent has two
//! commands going at once, the directory is the one reported by the most recently *started*
//! command that has finished: a slow command started earlier never moves an agent back.
//!
//! A persistent interactive shell was the alternative. It interleaves commands, needs prompt
//! markers to find where one ends, and would promise a continuity (variables, venvs) that a second
//! concurrent command, a kill or a restart cannot keep.
//!
//! # What a command gets
//!
//! - **Its own session and process group.** The PTY makes the command's `bash` a session leader,
//!   so its pid is the group's id. [`Jobs::kill`] signals the **whole group** — `SIGTERM`, then
//!   `SIGKILL` after [`Limits::kill_grace`] — not one pid. The leader is not reaped until that
//!   sequence is over, so the group id cannot be reused under the second signal. A process that
//!   leaves the group on purpose (`setsid`, a daemon) is beyond it, as with any terminal.
//! - **A built environment, not an inherited one:** `HOME`, `USER`, `PATH`, `LANG`,
//!   `TERM=xterm-256color` and `PWD`, and nothing from the shell's or the harness's own
//!   environment (bash adds `SHLVL` and `_` itself). See [`Environment`].
//! - **The directory is not a sandbox.** A command can read whatever the person can. The guard is
//!   the grade on the action that starts it (`agent_run` is `sensitive`) and #116's one rule on
//!   every door; a Landlock profile per agent is a later step.
//!
//! # Output
//!
//! Every byte goes three ways: to a vt100 emulator (what the card draws, and what the answer's
//! `tail` is read from — rendered text, so progress bars and colour codes do not reach a model),
//! to a per-job buffer that keeps the first and the last megabyte of [`Limits::retained_bytes`]
//! with a marker saying how much fell between, and to [`Jobs::on_output`] as raw terminal bytes
//! for the view. The emulator draws from cells: OSC 52 clipboard writes, hyperlinks and title
//! changes from a command do nothing.
//!
//! # A command that asks
//!
//! `sudo`'s password, `ssh`'s host key, a `read -p`: a job whose output has been silent for
//! [`Limits::silence`] while a process in its foreground group sits in `read(2)` on the terminal
//! reports `waiting_for_input: true`, and a wait on it returns early rather than hanging for the
//! rest of its budget. Best effort, from `/proc/<pid>/syscall` (or `wchan`), so an agent never just
//! looks hung. The person answers by typing into the card ([`Jobs::type_input`]); an agent can
//! answer with [`Jobs::input`].
//!
//! # Whose job it is
//!
//! Job ids are 128 random bits, and every call that names one names an agent too: a job of another
//! agent is refused. The agent is never taken from a caller's word — see [`AgentResolver`].
//!
//! # When the shell goes
//!
//! Dropping the last [`Jobs`] (or [`Jobs::shutdown`]) kills every running group. If the shell dies
//! without running either, the PTY masters close with it and the kernel hangs up each command's
//! terminal, which sends `SIGHUP` to its foreground group — the default terminal behaviour, and
//! enough for everything that has not asked to survive a hangup.

mod identity;
mod job;
mod jobs;
mod proc;
mod retained;

pub use identity::{descends_from, AgentResolver, HostTokens, Lookup, NoAgents, TokenTable, NO_AGENT};
pub use jobs::{
    Environment, FinishSink, JobId, JobState, JobSummary, Jobs, Limits, OutputSink, RunAnswer,
    DEFAULT_WAIT,
};
pub use retained::dropped_marker;

/// Which of the person's agents something belongs to — the harness wire's own type.
pub use yantrik_harness::event::AgentId;

/// The emulator the view draws a job's screen with ([`Jobs::with_screen`]).
pub use vt100;
