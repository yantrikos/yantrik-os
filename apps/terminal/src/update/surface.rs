//! The four actions a mind has: what they check, what they send to the shell, and what they are
//! told. A refusal is said twice: to the caller, and on the notice line the person sees.
//!
//! The input is queued for the shell here, on the UI thread, in the order it was asked for: that
//! is a bounded channel and a one-byte wake-up, and the session's own worker is what writes the
//! PTY. What the shell does with it is waited for on a worker (`wait.rs`) and told in the answer.

use std::path::PathBuf;

use yantrik_app::effects::Effects;

use super::{frame, refuse, tabs};
use crate::command::{Command, Why};
use crate::model::{tabs_full, Model};
use crate::wait::{self, Sent, WAIT};
use crate::Terminal;

type Fx = Effects<Terminal>;

const NO_SHELL: &str = "No shell is open in this Terminal; `new_tab` starts one.";

pub fn run(m: &mut Model, fx: &mut Fx, command: String) {
    let command = command.trim().to_string();
    if command.is_empty() {
        return refuse(m, fx, "`run` needs `command`: one command line for the active shell.");
    }
    let Some(tab) = m.tab() else {
        return refuse(m, fx, NO_SHELL);
    };
    if !tab.session.alive() {
        return refuse(m, fx, "The shell in this tab has exited; `new_tab` starts another.");
    }
    let (id, session) = (tab.id, tab.session.clone());
    let before = session.revision();
    let screen = session.snapshot().text;
    session.set_scrollback(0);
    if let Err(e) = session.write(format!("{command}\r").as_bytes()) {
        return refuse(m, fx, e);
    }
    frame::mark_due(m, fx);
    fx.spawn_or(Command::AnswerLost, move |_| wait::answer(session, id, before, Sent::Line { command, screen }, WAIT));
}

pub fn send_input(m: &mut Model, fx: &mut Fx, text: String) {
    if text.is_empty() {
        return refuse(m, fx, "`send_input` needs `text`: the exact characters to send to the shell.");
    }
    let Some(tab) = m.tab() else {
        return refuse(m, fx, NO_SHELL);
    };
    // What was running is the last probe's word, a tenth of a second old at most while a job runs.
    let (id, session, was_running) = (tab.id, tab.session.clone(), tab.facts.running.clone());
    let before = session.revision();
    if let Err(e) = session.write(text.as_bytes()) {
        return refuse(m, fx, e);
    }
    frame::mark_due(m, fx);
    let count = text.len();
    fx.spawn_or(Command::AnswerLost, move |_| {
        wait::answer(session, id, before, Sent::Bytes { count, was_running }, WAIT)
    });
}

pub fn open_directory(m: &mut Model, fx: &mut Fx, given: String) {
    let given = given.trim().to_string();
    if given.is_empty() {
        return refuse(m, fx, "`open_directory` needs `directory`: an absolute path to a folder.");
    }
    let dir = PathBuf::from(&given);
    if !dir.is_absolute() {
        return refuse(m, fx, format!("`directory` has to be an absolute path; `{given}` is not one."));
    }
    if let Some(full) = tabs_full(m.tabs.len() + m.spawning) {
        return refuse(m, fx, full);
    }
    tabs::start(m, fx, Why::Directory(given), Some(dir));
}

/// A worker that waited for a shell never answered: a panic, or the pool at its bound.
pub fn lost(m: &mut Model, fx: &mut Fx) {
    refuse(m, fx, "Terminal did not answer. The input may have been sent; look at `describe` before trying again.");
}
