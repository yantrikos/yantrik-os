//! What the Mind may ask Terminal by name: the same four actions as before, with the same names,
//! descriptions, parameters and grades, in the same order.
//!
//! Four actions were the best-described of the three apps a mind uses most, and still answered
//! nothing that had been observed: `run` answered `{"accepted": true, "completed": false,
//! "shell_pid": ...}` while the envelope around it said `settled: true`, `send_input` answered
//! `{"accepted": true}`, and `new_tab` answered `{"tabs": n}` whether or not a tab had opened,
//! because the tab cap returned in silence. They answer from what the shell did now (`wait.rs`,
//! `answers.rs`).
//!
//! # Grades
//!
//! Every action is `sensitive` and none is raised: `run` hands a line to the interactive shell the
//! person is looking at, which is the whole of what a terminal is for, and a `dangerous` grade
//! would put it above the shipped `sensitive` ceiling and leave the app publishing nothing a mind
//! could use. What the command itself may destroy is the machine's ceiling to decide
//! (`tool_permission` in settings.yaml), not this app's, and the person can see and interrupt
//! every line that arrives. `run` and `send_input` are open-ended: the line is whatever it is
//! given, a line ending in a newline at the shell's prompt is a command, so each asks once in
//! ask, auto and bypass, and a session rule answers the rest (29 September 2026, 51c4e224).
//! `new_tab` and `open_directory` are not: they start a shell and run nothing they are given.
//! Nothing here opens anything to an agent that was shut to it.
//!
//! # Settling
//!
//! `run` and `send_input` declare that they only start the work, as they always did. Every action
//! is answered when its own command has landed, in the command that carries it.

use std::time::Duration;

use serde_json::Value;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Args, Intent, Intents, Settle};

use crate::command::Command;
use crate::model::{Model, MAX_TABS};
use crate::Terminal;

/// How long an action may take to land before the Mind is told it did not. A shell is given under
/// a second to answer and a tab a moment to start.
pub const WITHIN: Duration = Duration::from_secs(8);

/// One action, with the one sentence a reader who cannot see the screen needs.
///
/// A description that is only the action's name, or too short to say what it does and to which
/// shell, stops the app before it serves and inside the tests.
fn act(name: &'static str, sentence: impl Into<String>) -> Action {
    let sentence = sentence.into();
    assert!(
        sentence.len() >= 20 && !sentence.starts_with("Terminal:"),
        "terminal action `{name}` was given a placeholder description: {sentence:?}"
    );
    Action::new(name, &sentence)
}

/// One argument, with the format it takes and what leaving it out means. Required by default.
fn arg(name: &'static str, sentence: &'static str) -> Param {
    assert!(sentence.len() >= 15, "terminal argument `{name}` was given no usable description: {sentence:?}");
    Param::text(name).describe(sentence)
}

/// The text of one argument as it was given. A blank one is the update's to refuse, in the
/// sentence the window shows too.
fn text(a: &Args<'_>, name: &str) -> String {
    a.raw().get(name).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn table(table: &mut Intents<Terminal>) {
    for (action, parse) in actions() {
        table.add(Intent::new(action, move |a| Ok(parse(a))).settles(Settle::Explicit { within: WITHIN }).reply(answer));
    }
}

/// What an intent answers: what `update` wrote in the command that settled it.
fn answer(m: &Model) -> Value {
    m.answer.clone()
}

type Parse = fn(&Args<'_>) -> Command;

/// The declarations, kept apart from the commands so a test can read exactly what a caller is
/// shown.
pub fn actions() -> Vec<(Action, Parse)> {
    vec![
        (
            act(
                "run",
                "Type a command line into the active shell and press Return. It does not wait for \
                 the command to finish: the answer carries what the shell printed in the moment \
                 after, `running` names anything still going, and `describe` shows the rest as it \
                 arrives. There is no exit status \u{2014} ask for one with `; echo exit=$?`.",
            )
            .risk("sensitive")
            .defers()
            .open_ended()
            .arg(arg(
                "command",
                "One command line, exactly as it would be typed; a newline is added. It is \
                 interpreted by the shell, so pipes, redirection and `cd` all work, and `cd` moves \
                 this tab for every command after it.",
            )),
            |a| Command::Run(text(a, "command")),
        ),
        (
            act(
                "send_input",
                "Send raw bytes to the active shell without pressing Return \u{2014} for answering a \
                 prompt a running command is waiting on, or sending a control character like \
                 \\u0003 (Ctrl-C) to interrupt it.",
            )
            .risk("sensitive")
            .defers()
            .open_ended()
            .arg(arg(
                "text",
                "The exact characters to send, with no newline added: end it with \\n to submit a \
                 line. \\u0003 interrupts, \\u0004 is end-of-input. Up to 64 KiB.",
            )),
            |a| Command::SendInput(text(a, "text")),
        ),
        (
            act(
                "new_tab",
                format!(
                    "Start another interactive shell in a new tab, in the same directory as the tab \
                     that is active now, and make the new one active. Up to {MAX_TABS} tabs."
                ),
            )
            .risk("sensitive"),
            |_| Command::NewTab,
        ),
        (
            act(
                "open_directory",
                "Start an interactive shell in a new tab whose working directory is this path, make \
                 it active, and bring the Terminal window to the front.",
            )
            .risk("sensitive")
            .arg(arg(
                "directory",
                "An absolute path to an existing folder; symlinks are resolved. It is refused if it \
                 is relative, missing, or not a folder.",
            )),
            |a| Command::OpenDirectory(text(a, "directory")),
        ),
    ]
}
