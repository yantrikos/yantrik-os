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
use crate::session::Snapshot;
use crate::wait::{self, Sent, WAIT};
use crate::Terminal;

type Fx = Effects<Terminal>;

const NO_SHELL: &str = "No shell is open in this Terminal; `new_tab` starts one.";
const MID_LINE: &str = "Someone is mid-line at this shell's prompt; the command was not sent. \
`send_input` with a leading `\\r` starts a clean line, or call `run` again once Return is pressed.";

/// Whether the person has typed into this shell and not pressed Return. Splicing a command line
/// onto that leaves the shell one line made of both, so `run` refuses it instead.
///
/// Nothing is running, so the bottom of the screen is the prompt line: what the shell wrote, then
/// the person's own characters. The cursor is no help — a shell keeps it right behind whatever
/// sits at the prompt, Return pressed or not — so the line is looked for by its prompt: every
/// prompt this image sets ends in `$ ` (`config/shell/bashrc`), and a plain `sh` or a root shell
/// ends in `$ ` or `# `.
///
/// Which is a guess, and it guesses at the person's expense in both directions. The screen the vt
/// keeps is rows, and a row says nothing about where the line it holds ended, so the bottom row is
/// read together with any row above it that the terminal ran out of room in: such a row is one it
/// filled, which is the only trace a wrap leaves.
///
/// A prompt this app does not know is read as a clean line and never refused over — a fish prompt
/// draws no `$ ` and no `# `, so a person mid-line in fish is typed on top of, which is the bug
/// this is there for. What is read instead is a line that ends at a known prompt with something
/// after it, and the mistake left over is a bottom row that is no prompt at all but sits under a
/// row the terminal filled which carried a prompt: output ending in a dollar sign is refused as
/// half-typed. Refusing and being asked again costs the caller a turn; typing into the person's
/// line does not untype it.
fn half_typed(snap: &Snapshot) -> bool {
    let (_rows, cols) = snap.size;
    // The rows as the vt kept them, bottom one first, each cut back to the text on it so that a
    // row is measured by what it shows and not by the room it has; what a trim leaves of an empty
    // screen.
    let mut above = snap.text.trim_end().split('\n').map(str::trim_end);
    let mut line = above.next_back().unwrap_or("").to_string();
    // Wide characters and a zero-width terminal both make the width a rough one; the wrap is only
    // ever missed, never invented, since a row is only joined to this line when it is at least a
    // terminal's width long.
    let wide = usize::max(usize::from(cols), 1);
    while let Some(row) = above.next_back() {
        if row.chars().count() < wide {
            break;
        }
        line.insert_str(0, row);
    }
    match ["$ ", "# "].into_iter().filter_map(|mark| line.rfind(mark)).max() {
        Some(at) => !line[at + 2..].trim().is_empty(),
        None => false,
    }
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
    // The same snapshot that is kept for telling what the shell answered says whether the person
    // is mid-line at this prompt; only a shell with nothing running sits at one.
    let (id, session, at_prompt) = (tab.id, tab.session.clone(), tab.facts.running.is_none());
    let before = session.revision();
    let snap = session.snapshot();
    if at_prompt && half_typed(&snap) {
        return refuse(m, fx, MID_LINE);
    }
    let screen = snap.text;
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

#[cfg(test)]
mod tests {
    use super::half_typed;
    use crate::session::Snapshot;

    /// A shell at rest, as `run` is handed it: the screen the vt keeps, one row per line of this
    /// text, each with its trailing spaces cut and a line wrapped past the terminal's width left
    /// as the two rows it became. The cursor is not filled in because `half_typed` does not read
    /// it.
    fn shell(text: &str) -> Snapshot {
        Snapshot {
            runs: Vec::new(),
            cursor: (0, 0),
            cursor_visible: true,
            alive: true,
            exit: None,
            error: None,
            scrollback: 0,
            size: (24, 100),
            text: text.into(),
        }
    }

    #[test]
    fn half_typed_is_the_line_the_person_stopped_in_the_middle_of() {
        // The prompt itself, and the prompt with the person's own characters after it.
        assert!(!half_typed(&shell("yantrik $")));
        assert!(half_typed(&shell("yantrik $ git che")));
        // A prompt a command has answered, and output with no prompt under it yet: the shell has
        // printed and not drawn the next prompt, and that text is nobody's half-typed line.
        assert!(!half_typed(&shell("yantrik $\nsrc\nyantrik $")));
        assert!(!half_typed(&shell("yantrik $\nsrc\nREADME")));
        // A root shell's prompt ends in `# `, and a prompt this app does not know is never
        // refused over, not even with a line after it.
        assert!(half_typed(&shell("root@node:~# apt")));
        assert!(!half_typed(&shell("yantrik > git che")));
        // A lone space at the prompt is cut with the rest of the line's trailing spaces, so it
        // reads as the clean prompt it is: the command lands as its own line either way.
        assert!(!half_typed(&shell("yantrik $  ")));
        // An empty screen, and a shell that has only printed: nothing at a prompt, nothing typed.
        assert!(!half_typed(&shell("")));
        assert!(!half_typed(&shell("\n\n")));
    }

    #[test]
    fn a_half_line_the_terminal_wrapped_is_still_one_line() {
        // A terminal 100 columns wide, so the prompt and the first hundred columns of the line are
        // one row and what the person typed on past the edge is the row under it. Rows are all
        // `half_typed` is given, and a row it filled is the only trace a wrap leaves.
        let to_the_edge = format!("yantrik $ git che{}", "x".repeat(83));
        assert_eq!(to_the_edge.chars().count(), 100);
        assert!(half_typed(&shell(&format!("{to_the_edge}\nck -m fix"))));
        // Two wraps deep, the prompt three rows up.
        let twice = format!("{to_the_edge}\n{}", "y".repeat(100));
        assert!(half_typed(&shell(&format!("{twice}\nck -m fix"))));
        // A row that stopped short of the edge ended its own line, so what is under it is not part
        // of it: an answer printed at the bottom of the screen stays nobody's half-typed line.
        assert!(!half_typed(&shell("yantrik $ echo hi\nsrc\nREADME")));
        // The same screen as a vt that keeps a row filled to its width would hand over. Measuring
        // what a row shows instead of the room it has is what keeps this the two rows they were.
        let padded = |row: &str| format!("{row}{}", " ".repeat(100 - row.chars().count()));
        assert!(!half_typed(&shell(&format!("{}\n{}", padded("yantrik $ echo hi"), padded("src")))));
    }

    /// What the guess costs, written down where it can be read: output that ran exactly to the
    /// edge of the window is shaped like a line typed to it, and is refused for it.
    #[test]
    fn a_row_that_ran_to_the_edge_is_read_as_a_line_typed_to_it() {
        let to_the_edge = format!("yantrik $ echo {}", "x".repeat(85));
        assert_eq!(to_the_edge.chars().count(), 100);
        // The row under it is what the command printed, and no prompt on it: still one line as far
        // as the rows go, so `run` holds off and the caller is asked to call again.
        assert!(half_typed(&shell(&format!("{to_the_edge}\ntotal 8"))));
    }
}
