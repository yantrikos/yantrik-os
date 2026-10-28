//! Waiting for a shell to begin answering, on a worker, and saying what it did.
//!
//! A PTY write returns as soon as the bytes are queued: nothing has run yet. So `run` and
//! `send_input` declare `defers`, the envelope says `settled: false`, and the caller is told to
//! look again rather than reporting a command as finished the moment it was typed. But
//! "accepted" alone reports nothing observed. These wait for the session's revision to move,
//! which is the shell echoing or printing, and the answer carries what came back. They do not
//! wait for the command to finish: `sleep 30` takes thirty seconds and one call gives the shell
//! under a second.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::answers::{appeared, shell_body};
use crate::command::{Command, Reply};
use crate::probe;
use crate::session::Session;

/// The most a call waits for the shell to answer.
pub const WAIT: Duration = Duration::from_millis(900);

/// What was sent, and what the answer should say about it.
pub enum Sent {
    /// A command line and the screen as it was when it was typed.
    Line { command: String, screen: String },
    /// Raw bytes, and what was running in the shell before them.
    Bytes { count: usize, was_running: Option<String> },
}

/// Wait, bounded, for the shell to begin answering, and say whether it did.
///
/// Given the screen as it was (`shown`), the wait goes on, inside the same bound, until the text
/// on it has changed and not merely the revision. The first thing a shell sends back after a line
/// is often a cursor move or a mode switch: the revision moved, `run` answered at once, and its
/// `output` said nothing had appeared while the echo was a few milliseconds behind (a CI run on
/// main caught exactly that). `send_input` passes no screen: a key sent to a program may rightly
/// change no text at all.
pub fn responded(session: &Session, before: u64, shown: Option<&str>, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    let mut seen = before;
    loop {
        let now = session.revision();
        if now != seen {
            seen = now;
            match shown {
                // Snapshots only when something moved: the screen is read under the parser's
                // lock, which the reader thread needs too.
                Some(shown) if session.snapshot().text == shown => {}
                _ => return true,
            }
        }
        if Instant::now() >= deadline {
            return seen != before;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The worker for one `run` or `send_input`: it waits for the shell, then reads it.
pub fn answer(session: Arc<Session>, tab: u64, before: u64, sent: Sent, wait: Duration) -> Command {
    let mut body;
    match sent {
        Sent::Line { command, screen } => {
            let answered = responded(&session, before, Some(&screen), wait);
            body = shell_body(&session, &probe::read(&session));
            body["sent"] = json!(command);
            body["shell_answered"] = json!(answered);
            // What came back in the time this call has, which is not the same as the command
            // being over: `running` says whether something is still going, and `settled: false`
            // on the envelope says this action never waits to find out.
            body["output"] = json!(appeared(&screen, &session.snapshot().text));
            body["finished"] = json!(false);
        }
        Sent::Bytes { count, was_running } => {
            let answered = responded(&session, before, None, wait);
            body = shell_body(&session, &probe::read(&session));
            body["sent_bytes"] = json!(count);
            body["shell_answered"] = json!(answered);
            body["was_running"] = json!(was_running);
        }
    }
    Command::Answered(Box::new(Reply { tab, body }))
}
