//! An app's own gate records what it runs unasked, with the shell (#1353, round 4 of #1357).
//!
//! The shell writes the audit line for every action it answers `not_needed`. A program that
//! reaches an app's socket directly never meets the shell's request path, so the app's gate is the
//! one place that knows it ran an action without a card. In either bypass, when [`decide`] lets a
//! call through that the `ask` mode would have refused and no grant stands behind it, the gate
//! hands the shell the whole call ([`record`]) before it runs, and the shell writes the line. If
//! the shell cannot be reached, or cannot write the line, the call is refused: what cannot be
//! recorded does not run. A call a person allowed (a grant) is not here: a card was the record.

use crate::gate::{decide, Authority, Mode};
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

/// The action the shell answers with a line of its own: `record_gate_decision`.
pub const ACTION: &str = "record_gate_decision";

/// How long a gate waits for the shell to say it wrote the line.
const ROUNDTRIP: Duration = Duration::from_secs(2);

/// What a gate hands the shell about one call it is about to run unasked.
pub struct Record<'a> {
    /// The app whose gate let it through.
    pub app: &'a str,
    pub action: &'a str,
    pub grade: &'a str,
    /// `bypass` or `bypass_all`.
    pub mode: &'a str,
    /// The whole arguments, exactly as the call carried them.
    pub args: &'a Value,
}

type Writer = dyn Fn(&Record<'_>) -> Result<(), String> + Send + Sync;

static WRITER: OnceLock<Box<Writer>> = OnceLock::new();

/// Install how this process writes the line itself. The shell calls it once; every other process
/// asks the shell.
pub fn write_with(write: impl Fn(&Record<'_>) -> Result<(), String> + Send + Sync + 'static) {
    let _ = WRITER.set(Box::new(write));
}

/// Whether `authority` lets this call run unasked: in either bypass, with no grant, where the `ask`
/// mode (no rule of any kind, the same ceiling, the same hold) would have refused it.
pub fn is_unasked(authority: &Authority, app: &str, action: &str, graded: &str, purpose: &str) -> bool {
    if authority.granted || !matches!(authority.mode.name.as_str(), "bypass" | "bypass_all") {
        return false;
    }
    let strict = Authority { ceiling: authority.ceiling.clone(), mode: Mode::named("ask"), granted: false, asks_above: authority.asks_above };
    decide(&strict, app, action, graded, purpose).is_err()
}

/// [`decide`], and the record of the call when it runs unasked. `Err` carries the refusal, or that
/// the shell could not be made to write the line.
pub fn decide_recorded(authority: &Authority, app: &str, action: &str, graded: &str, purpose: &str, args: &Value) -> Result<(), String> {
    decide_with(authority, app, action, graded, purpose, args, record)
}

/// [`decide_recorded`] with the writer given, for the tests.
pub fn decide_with(
    authority: &Authority,
    app: &str,
    action: &str,
    graded: &str,
    purpose: &str,
    args: &Value,
    write: impl Fn(&Record<'_>) -> Result<(), String>,
) -> Result<(), String> {
    decide(authority, app, action, graded, purpose)?;
    if is_unasked(authority, app, action, graded, purpose) {
        write(&Record { app, action, grade: graded, mode: &authority.mode.name, args })
            .map_err(|why| format!("NOT RUN: {app}.{action} would run without asking, and that has to be written down first, which could not be done ({why})."))?;
    }
    Ok(())
}

/// Have the line written: in the shell itself, or by asking it over its socket.
pub fn record(r: &Record<'_>) -> Result<(), String> {
    match WRITER.get() {
        Some(write) => write(r),
        None => ask_the_shell(r),
    }
}

#[cfg(unix)]
fn ask_the_shell(r: &Record<'_>) -> Result<(), String> {
    let reply = crate::SyncRpcClient::for_service("app-shell")
        .with_timeout(ROUNDTRIP)
        .expecting_peer(crate::owner::must_be_the_shell)
        .call(
            "app.act",
            serde_json::json!({ "action": ACTION, "args": {
                "app": r.app, "action": r.action, "grade": r.grade, "mode": r.mode,
                "args_json": r.args.to_string(),
            } }),
        )
        .map_err(|e| format!("the shell did not answer ({})", e.message))?;
    recorded_in_reply(&reply)
}

#[cfg(not(unix))]
fn ask_the_shell(_r: &Record<'_>) -> Result<(), String> {
    Err("there is no shell socket to ask on this platform".into())
}

/// The shell's answer: `{"result": {"written": true}}`. Anything else is a refusal.
pub fn recorded_in_reply(reply: &Value) -> Result<(), String> {
    match reply.get("result").and_then(|r| r.get("written")).and_then(Value::as_bool) {
        Some(true) => Ok(()),
        _ => Err(reply.get("error").map(|e| e.to_string()).unwrap_or_else(|| "the shell did not say it wrote the line".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    fn at(mode: &str, ceiling: &str) -> Authority {
        Authority { ceiling: ceiling.into(), mode: Mode::named(mode), granted: false, asks_above: None }
    }
    const RUN: &str = "Run a command. What it runs can do anything you can.";
    const GONE: &str = "Take an event off the calendar. It is not recoverable";

    fn run(a: &Authority, grade: &str, purpose: &str, fail: bool) -> (Result<(), String>, Vec<String>) {
        let heard = RefCell::new(Vec::new());
        let got = decide_with(a, "terminal", "run", grade, purpose, &json!({ "command": "ls" }), |r| {
            heard.borrow_mut().push(format!("{}:{}", r.mode, r.args));
            if fail { Err("no shell".into()) } else { Ok(()) }
        });
        (got, heard.into_inner())
    }

    #[test]
    fn either_bypass_writes_the_line_for_a_command_and_for_what_cannot_be_undone() {
        for mode in ["bypass", "bypass_all"] {
            let (got, heard) = run(&at(mode, "dangerous"), "sensitive", RUN, false);
            assert!(got.is_ok() && heard.len() == 1, "{mode}: {got:?} {heard:?}");
            assert!(heard[0].contains("\"command\":\"ls\""), "the whole call is handed over");
        }
        let (got, heard) = run(&at("bypass_all", "dangerous"), "sensitive", GONE, false);
        assert!(got.is_ok() && heard.len() == 1, "full bypass runs what cannot be undone, recorded");
    }

    #[test]
    fn no_shell_no_run() {
        for mode in ["bypass", "bypass_all"] {
            let (got, _) = run(&at(mode, "dangerous"), "sensitive", RUN, true);
            assert!(got.unwrap_err().contains("NOT RUN"), "{mode}");
        }
    }

    #[test]
    fn nothing_else_is_recorded_here() {
        // A read, which the ask mode runs: nothing to record, and so no shell needed.
        let (got, heard) = run(&at("bypass_all", "dangerous"), "safe", RUN, true);
        assert!(got.is_ok() && heard.is_empty());
        // Modes that ask are not unasked: ask, auto and plan never get here with a command.
        for mode in ["ask", "auto", "plan"] {
            let (got, heard) = run(&at(mode, "dangerous"), "sensitive", RUN, false);
            assert!(heard.is_empty(), "{mode}");
            assert!(got.is_err() || !is_unasked(&at(mode, "dangerous"), "terminal", "run", "sensitive", RUN), "{mode}");
        }
        // A grant is a person's Allow: a card was the record.
        let mut granted = at("bypass", "dangerous");
        granted.granted = true;
        let (got, heard) = run(&granted, "sensitive", RUN, true);
        assert!(got.is_ok() && heard.is_empty());
        // A refusal is a refusal, and writes nothing.
        let (got, heard) = run(&at("bypass_all", "safe"), "sensitive", RUN, false);
        assert!(got.is_err() && heard.is_empty(), "the ceiling comes first");
    }

    #[test]
    fn the_reply_has_to_say_it_was_written() {
        assert!(recorded_in_reply(&json!({ "result": { "written": true } })).is_ok());
        assert!(recorded_in_reply(&json!({ "result": { "written": false } })).is_err());
        assert!(recorded_in_reply(&json!({ "result": {} })).is_err());
        assert!(recorded_in_reply(&json!({ "error": "no" })).is_err());
    }
}
