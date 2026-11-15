//! The text reader, kept running between reads (#1256).
//!
//! `read_screen` used to start `yantrik-ocr` for each read. The reader read every line on the
//! display and went away with everything it had learned: nine seconds on an old machine,
//! whatever had changed. Kept running, it remembers what each line said (its own `memory`), so
//! a read of a display that changed a little costs a little: measured on the same machine, a
//! terminal that scrolled by one line is read again in 0.6 s.
//!
//! What "kept running" must not mean:
//!
//! - **Not kept for ever.** The reader leaves by itself two minutes after it was last asked,
//!   and is started again by the next read.
//! - **Not kept across a lock or Private mode.** [`forget`] ends it, and with it everything it
//!   remembers of what was on screen. It never waits for a read in flight: the read fails.
//! - **Not trusted to answer.** A read has a time limit. A reader that passes it is ended, and
//!   the next read starts a new one.
//! - **Not let loose on memory.** Its address space has the ceiling it always had.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Address space the reader may take. It needs a few hundred MB for a desktop; this is the
/// ceiling that keeps a pathological image from taking the session with it.
const READER_MEMORY: u64 = 3 << 30;

/// How long the reader stays without being asked, in seconds. It is told this, and keeps to it
/// itself: the shell does not have to be alive to end it.
const IDLE_SECS: &str = "120";

/// One running reader.
struct Kept {
    child: Arc<Mutex<Child>>,
    requests: ChildStdin,
    answers: Receiver<String>,
}

/// The reader, when one is running.
#[derive(Default)]
pub struct Keeper {
    /// Held for the whole of a read: one read at a time.
    kept: Mutex<Option<Kept>>,
    /// The same child, for [`Keeper::forget`], which must never wait behind a read.
    child: Mutex<Option<Arc<Mutex<Child>>>>,
}

/// The desktop's one reader.
static READER: Keeper = Keeper { kept: Mutex::new(None), child: Mutex::new(None) };

fn held<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|e| e.into_inner())
}

/// The reader as it is started for the desktop: serving, with its address-space ceiling.
fn serving(bin: &Path) -> Command {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(bin);
    cmd.args(["--idle-secs", IDLE_SECS, "--serve"]);
    // SAFETY: setrlimit is async-signal-safe, and nothing else runs between fork and exec here.
    unsafe {
        cmd.pre_exec(|| {
            let limit = libc::rlimit { rlim_cur: READER_MEMORY, rlim_max: READER_MEMORY };
            if libc::setrlimit(libc::RLIMIT_AS, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd
}

fn start(mut cmd: Command) -> Result<Kept, String> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start the text reader: {e}"))?;
    let (Some(requests), Some(out)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("the text reader started with nothing to talk to it through".into());
    };
    let (tx, answers) = mpsc::channel();
    // Ends when the reader's output closes, which is when the reader has gone.
    let reading = std::thread::Builder::new().name("text-reader".into()).spawn(move || {
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    if let Err(e) = reading {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("could not listen to the text reader: {e}"));
    }
    Ok(Kept { child: Arc::new(Mutex::new(child)), requests, answers })
}

/// End a reader and collect it. Its memory of what was on screen goes with it.
fn end(kept: Kept) {
    let mut child = held(&kept.child);
    let _ = child.kill();
    let _ = child.wait();
}

impl Keeper {
    /// Ask the reader one thing, starting it with `command` when none is running. The answer is
    /// the reader's own JSON; an answer that is an error, no answer within `limit`, and a
    /// reader that stops are all errors, and the last two end the reader.
    pub fn ask(&self, command: impl FnOnce() -> Command, request: &serde_json::Value, limit: Duration) -> Result<serde_json::Value, String> {
        let mut kept = held(&self.kept);
        // One that left by itself, or was ended by `forget`, is collected and replaced. A reader
        // that `forget` has signalled may not have died yet, so it is known by `forget` having
        // taken its handle, not by whether it is still there.
        let forgotten = held(&self.child).is_none();
        let gone = kept.as_ref().is_some_and(|reader| forgotten || !matches!(held(&reader.child).try_wait(), Ok(None)));
        if gone {
            if let Some(reader) = kept.take() {
                end(reader);
            }
        }
        if kept.is_none() {
            let started = start(command())?;
            *held(&self.child) = Some(Arc::clone(&started.child));
            *kept = Some(started);
        }
        let Some(reader) = kept.as_mut() else {
            return Err("the text reader is not running".into());
        };
        let sent = writeln!(reader.requests, "{request}").and_then(|()| reader.requests.flush());
        let answer = match sent {
            Ok(()) => reader.answers.recv_timeout(limit).map_err(|why| match why {
                RecvTimeoutError::Timeout => format!("the text reader did not finish in {} s", limit.as_secs()),
                RecvTimeoutError::Disconnected => "the text reader stopped before it answered".to_string(),
            }),
            Err(e) => Err(format!("the text reader would not take the request: {e}")),
        };
        let line = match answer {
            Ok(line) => line,
            Err(why) => {
                // Late or gone: not a reader to ask again.
                *held(&self.child) = None;
                if let Some(reader) = kept.take() {
                    end(reader);
                }
                return Err(why);
            }
        };
        let answer: serde_json::Value = serde_json::from_str(&line).map_err(|e| format!("the text reader answered nonsense: {e}"))?;
        match answer["error"].as_str() {
            Some(why) => Err(format!("the text reader could not read it: {why}")),
            None => Ok(answer),
        }
    }

    /// End the reader, and with it everything it remembers of what was on screen. Returns at
    /// once, whatever a read is doing: a read in flight fails.
    pub fn forget(&self) {
        if let Some(child) = held(&self.child).take() {
            // Only the signal. Whoever holds `kept` collects it: the read in flight, or the
            // next read.
            let _ = held(&child).kill();
        }
    }
}

/// Ask the desktop's reader (the `yantrik-ocr` at `bin`) one thing.
pub fn ask(bin: &Path, request: &serde_json::Value, limit: Duration) -> Result<serde_json::Value, String> {
    READER.ask(|| serving(bin), request, limit)
}

/// End the desktop's reader: the screen locked, or the person went private.
pub fn forget() {
    READER.forget();
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;
