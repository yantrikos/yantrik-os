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
/// Ctrl-U: the line-editor's kill line, which a shell clears the cursor's line with.
const KILL_LINE: &str = "\u{15}";
/// The last byte that is a control character rather than text.
const LAST_CONTROL: u8 = 0x1f;
/// How a prompt ends: a dollar, or a hash when the shell runs as root. The image's own shell
/// config ends every prompt it sets with one of these and a space, and a plain `sh` and `bash` do.
const PROMPT_ENDS: [char; 2] = ['$', '#'];
/// How much of the person's line a refusal quotes.
const QUOTE: usize = 40;

/// What the person is typing where the call arrived — the screen's text and cursor, as a snapshot
/// holds them: what the row the cursor is on holds past the shell's prompt. `None` says the prompt
/// is empty, so anything written there starts a line. A snapshot already trims a row's trailing
/// spaces, so an empty prompt is the bare `yantrik $` and not that plus a space.
///
/// The prompt is not known here, so it is found the way a person finds it: the last `$ ` or `# `
/// in the row, or a `$`/`#` the row ends with. A prompt shaped otherwise (a `%` one, a fish-style
/// one, a person's own) reads as a line in progress, which is the safe way to be wrong — a `run`
/// clears a line nobody typed, `send_input` refuses — and never splices their words with the
/// agent's. A line ending in `$` or `#` at such a prompt reads as no line, and joins as it did
/// before; that is the one shape where the two are indistinguishable.
fn line_in_progress(text: &str, cursor: (u16, u16)) -> Option<String> {
    let row = text.split('\n').nth(cursor.0 as usize)?.trim_end();
    // The prompt's last character, nearest the end of the row: one the row ends with, where the
    // space after it was trimmed, or one with a space after it, which is where the line begins. A
    // dollar that is part of a variable (`echo $HOME`) is neither, so it is passed over.
    let at = row
        .rmatch_indices(PROMPT_ENDS)
        .map(|(at, _)| at)
        .find(|at| *at + 1 == row.len() || row.as_bytes()[*at + 1] == b' ');
    let typed = match at {
        Some(at) => row[at + 1..].trim(),
        None => row,
    };
    (!typed.is_empty()).then(|| typed.to_string())
}

/// The sentence a caller gets when raw bytes are refused for the sake of the person's line.
fn typing(line: &str) -> String {
    let mut quoted: String = line.chars().take(QUOTE).collect();
    if quoted.chars().count() < line.chars().count() {
        quoted.push('…');
    }
    format!("The person is typing at this shell (`{quoted}`); send_input would land on their line. Ask again when it is clear.")
}

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
    let shot = session.snapshot();
    // The person's half-typed line is never joined to the agent's: Ctrl-U clears the prompt
    // first, so what runs is the line this call asked for and nothing else. Their words are
    // lost from the prompt, which the answer tells them with `replaced_line`.
    let replaced = line_in_progress(&shot.text, shot.cursor);
    let screen = shot.text;
    let kill = if replaced.is_some() { KILL_LINE } else { "" };
    session.set_scrollback(0);
    if let Err(e) = session.write(format!("{kill}{command}\r").as_bytes()) {
        return refuse(m, fx, e);
    }
    frame::mark_due(m, fx);
    fx.spawn_or(Command::AnswerLost, move |_| {
        let sent = Sent::Line { command, screen, replaced };
        wait::answer(session, id, before, sent, WAIT)
    });
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
    // Raw bytes are the escape hatch, so they must not splice into the person's line either:
    // only a control character goes through, since interrupting a shell is what it is for.
    if text.as_bytes()[0] > LAST_CONTROL {
        let shot = session.snapshot();
        if let Some(line) = line_in_progress(&shot.text, shot.cursor) {
            return refuse(m, fx, typing(&line));
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot's rows carry no trailing spaces, which is what these rows are written as.
    #[test]
    fn a_bare_prompt_or_an_empty_row_is_no_line_in_progress() {
        assert_eq!(line_in_progress("", (0, 0)), None, "nothing on the screen at all");
        assert_eq!(line_in_progress("$", (0, 1)), None, "a prompt with nothing after it");
        assert_eq!(line_in_progress("root@yantrik /mnt/code #", (0, 24)), None, "a root prompt, which ends in a hash");
        assert_eq!(line_in_progress("echo one\nyantrik $ echo ha", (1, 22)), Some("echo ha".to_string()), "the row the cursor is on, not the one above");
    }

    #[test]
    fn the_line_is_what_follows_the_prompt_and_no_more() {
        assert_eq!(line_in_progress("$ echo ha", (0, 9)), Some("echo ha".to_string()));
        assert_eq!(line_in_progress("/home/yantrik $ echo ha", (0, 23)), Some("echo ha".to_string()), "the last prompt, wherever it ends");
        assert_eq!(line_in_progress("root@yantrik /mnt/code # rm -rf /mnt/code", (0, 41)), Some("rm -rf /mnt/code".to_string()), "as root, the hash ends the prompt");
        // A dollar inside their own line is not a second prompt where it is part of a word.
        assert_eq!(line_in_progress("yantrik $ echo $HOME", (0, 20)), Some("echo $HOME".to_string()));
        // Their own `$ ` does read as a second prompt, so the quote is short. What matters is that
        // it is still a line: the whole row is cleared, and send_input still refuses.
        assert_eq!(line_in_progress("yantrik $ echo $ HOME", (0, 21)), Some("HOME".to_string()));
        assert_eq!(line_in_progress("output\nyantrik $\n$ echo ha", (2, 9)), Some("echo ha".to_string()), "rows above are what already ran");
        assert_eq!(line_in_progress("$ echo ha", (9, 0)), None, "a cursor on a row the screen does not have");
    }
}
