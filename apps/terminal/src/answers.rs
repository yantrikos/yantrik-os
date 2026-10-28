//! The answer each action is owed, built from its own result.
//!
//! An action's reply is read from the model when it settles, so each one settles in the command
//! that carries its own result: `m.answer` is written and `fx.settle` asked for in that one
//! command, and nothing can land between the two. What the shell did is read by the worker that
//! waited for it (`wait.rs`), about the shell the input went to, never about whichever tab the
//! person has in front of them when it lands. A command with no invocation (a key, a button)
//! never writes `m.answer`.

use serde_json::{json, Value};
use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use crate::command::Reply;
use crate::model::{Facts, Model};
use crate::session::Session;
use crate::Terminal;

type Fx = Effects<Terminal>;

/// Whether the command being applied owes a surface call its answer.
pub fn owed(fx: &Fx) -> bool {
    fx.invocation().is_some()
}

/// Settle the invocation this command runs under with `answer`, if it has one.
pub fn settle(m: &mut Model, fx: &mut Fx, answer: Value) {
    if owed(fx) {
        m.answer = answer;
        fx.settle(Ok(()));
    }
}

/// Refuse the call this command answers, in the sentence the window shows too.
pub fn fail(fx: &mut Fx, sentence: String) {
    if owed(fx) {
        fx.settle(Err(Refused::new("failed", sentence)));
    }
}

/// The last few lines the screen actually shows: what a person looking at it would read.
pub fn tail(text: &str, lines: usize) -> String {
    let kept: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
    kept[kept.len().saturating_sub(lines)..].join("\n")
}

/// What appeared on the screen after the command was sent: the lines the old screen did not have.
///
/// This is the closest thing to "the command's output" that an interactive shell can honestly
/// give in the time one call has. It is not the exit status: nothing short of waiting for the
/// command, or echoing a sentinel into the person's own terminal, can know that, so a caller
/// that needs one asks for it: `run command="make; echo exit=$?"`. If the screen scrolled, the
/// lines in common are gone and this is simply what is on it now.
pub fn appeared(before: &str, after: &str) -> String {
    let was: Vec<&str> = before.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
    let now: Vec<&str> = after.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
    let mut same = 0;
    while same < was.len().min(now.len()) && was[same] == now[same] {
        same += 1;
    }
    now[same..].join("\n")
}

/// What one shell now is, read back off the PTY and `/proc` rather than assumed.
pub fn shell_body(session: &Session, facts: &Facts) -> Value {
    let snap = session.snapshot();
    json!({
        "directory": facts.cwd.display().to_string(),
        "shell_pid": session.pid(),
        "alive": snap.alive,
        "running": facts.running,
        "shell_exit_code": snap.exit,
        "screen_tail": tail(&snap.text, 12),
        "error": snap.error,
    })
}

/// A worker's reply, with what only the model knows added at the moment it lands: where the tab
/// is among the tabs now (null if it was closed meanwhile), how many there are, the notice.
pub fn landed(m: &Model, reply: Reply) -> Value {
    let mut body = reply.body;
    body["tab"] = json!(m.index_of(reply.tab));
    body["tabs"] = json!(m.tabs.len());
    body["notice"] = json!(m.notice);
    body
}
