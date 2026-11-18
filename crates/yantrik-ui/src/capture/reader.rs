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
//!   A read takes the [`epoch`] when it is asked for, and `forget` moves it on. A read whose
//!   epoch is not the current one starts no reader, and one whose answer arrives after `forget`
//!   ends the reader it started and returns nothing: a read admitted before a lock cannot
//!   complete after it and leave a reader holding what it saw.
//! - **Not trusted to answer.** A read has a time limit. A reader that passes it is ended, and
//!   the next read starts a new one.
//! - **Not let loose on memory.** Its address space has the ceiling it always had.
//! - **Not mute.** What the reader says on its standard error is logged line by line (bounded,
//!   `reader_log`), and the last line is part of the error when it stops. What it reads off the
//!   screen is never logged.
//! - **Not confused.** Each request carries an id and an answer is taken only with that id, so
//!   a stray line on its output is never the answer to the next read.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::reader_log::each_line;

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
    /// The last thing it said on its standard error, and a signal that it has said all it will.
    said: Arc<Mutex<String>>,
    said_all: Receiver<()>,
}

/// The reader, when one is running.
#[derive(Default)]
pub struct Keeper {
    /// Held for the whole of a read: one read at a time.
    kept: Mutex<Option<Kept>>,
    /// The same child, for [`Keeper::forget`], which must never wait behind a read.
    child: Mutex<Option<Arc<Mutex<Child>>>>,
    /// Moved on by every [`Keeper::forget`].
    epoch: AtomicU64,
    /// The number of the last request. Ids start at 1.
    next: AtomicU64,
}

/// The desktop's one reader.
static READER: Keeper = Keeper { kept: Mutex::new(None), child: Mutex::new(None), epoch: AtomicU64::new(0), next: AtomicU64::new(0) };

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
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the text reader: {e}"))?;
    let (Some(requests), Some(out), Some(err)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("the text reader started with nothing to talk to it through".into());
    };
    let child = Arc::new(Mutex::new(child));
    let (tx, answers) = mpsc::channel();
    // Ends when the reader's output closes, which is when the reader has gone. Then it collects
    // the reader, so one that left by itself is not a zombie until the next read.
    let reaped = Arc::clone(&child);
    let reading = std::thread::Builder::new().name("text-reader".into()).spawn(move || listen(out, &tx, &reaped));
    let said = Arc::new(Mutex::new(String::new()));
    let (done, said_all) = mpsc::channel();
    let logged = Arc::clone(&said);
    let logging = std::thread::Builder::new().name("text-reader-log".into()).spawn(move || log(err, &logged, &done));
    if let Err(e) = reading.and(logging) {
        let mut child = held(&child);
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("could not listen to the text reader: {e}"));
    }
    Ok(Kept { child, requests, answers, said, said_all })
}

fn listen(out: std::process::ChildStdout, tx: &Sender<String>, child: &Mutex<Child>) {
    for line in BufReader::new(out).lines() {
        let Ok(line) = line else { break };
        if tx.send(line).is_err() {
            break;
        }
    }
    for _ in 0..200 {
        if !matches!(held(child).try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn log(err: std::process::ChildStderr, last: &Mutex<String>, done: &Sender<()>) {
    each_line(BufReader::new(err), |line| {
        tracing::warn!(target: "text-reader", said = %line, "the text reader said");
        *held(last) = line;
    });
    let _ = done.send(());
}

/// End a reader and collect it. Its memory of what was on screen goes with it.
fn end(kept: Kept) -> String {
    {
        let mut child = held(&kept.child);
        let _ = child.kill();
        let _ = child.wait();
    }
    // It has said all it will once its end of the pipe is closed. A grandchild could hold that
    // open, so not for long.
    let _ = kept.said_all.recv_timeout(Duration::from_millis(500));
    let last = held(&kept.said).clone();
    last
}

/// Why a read failed, and whether a reader that was already running when it was asked is the
/// likely reason (it left in the instant between the shell looking and the request arriving).
struct Failed {
    why: String,
    retry: bool,
}

fn failed(why: impl Into<String>) -> Failed {
    Failed { why: why.into(), retry: false }
}

impl Keeper {
    /// The epoch a read that is asked for now belongs to. [`Keeper::forget`] moves it on.
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// Ask the reader one thing, starting it with `command` when none is running. The answer is
    /// the reader's own JSON; an answer that is an error, no answer within `limit`, and a
    /// reader that stops are all errors, and the last two end the reader. `epoch` is
    /// [`Keeper::epoch`] as it was when the read was asked for: if it has moved on, by the time
    /// a reader would start or an answer is back, the read fails and no reader is left.
    pub fn ask(&self, command: impl FnOnce() -> Command, request: &serde_json::Value, limit: Duration, epoch: u64) -> Result<serde_json::Value, String> {
        self.ask_then(command, request, limit, epoch, || {})
    }

    /// [`Keeper::ask`], running `answered` once the reader's answer is in and before it is
    /// let out.
    fn ask_then(&self, command: impl FnOnce() -> Command, request: &serde_json::Value, limit: Duration, epoch: u64, answered: impl FnOnce()) -> Result<serde_json::Value, String> {
        let mut kept = held(&self.kept);
        let (mut command, mut answered) = (Some(command), Some(answered));
        let mut once = |kept: &mut Option<Kept>| self.once(kept, &mut command, request, limit, epoch, &mut answered);
        match once(&mut kept) {
            // A reader that was running left as the request came. Reads change nothing, so
            // one more try, with a reader that has just been started, is safe.
            Err(Failed { retry: true, .. }) => once(&mut kept).map_err(|f| f.why),
            done => done.map_err(|f| f.why),
        }
    }

    fn moved_on(&self, epoch: u64) -> Result<(), Failed> {
        if self.epoch() == epoch {
            Ok(())
        } else {
            Err(failed("the screen locked, or the person went private, while this read was waiting: nothing was read"))
        }
    }

    fn once<C: FnOnce() -> Command, A: FnOnce()>(&self, kept: &mut Option<Kept>, command: &mut Option<C>, request: &serde_json::Value, limit: Duration, epoch: u64, answered: &mut Option<A>) -> Result<serde_json::Value, Failed> {
        self.moved_on(epoch)?;
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
        let reused = kept.is_some();
        if !reused {
            let Some(command) = command.take() else {
                return Err(failed("the text reader is not running"));
            };
            let started = start(command()).map_err(failed)?;
            *held(&self.child) = Some(Arc::clone(&started.child));
            *kept = Some(started);
            // `forget` moves the epoch before it looks for a reader. A forget that came after
            // the check above and before the reader was known to it is seen here.
            if let Err(why) = self.moved_on(epoch) {
                self.drop_reader(kept);
                return Err(why);
            }
        }
        let Some(reader) = kept.as_mut() else {
            return Err(failed("the text reader is not running"));
        };
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let mut request = request.clone();
        request["id"] = id.into();
        let sent = writeln!(reader.requests, "{request}").and_then(|()| reader.requests.flush());
        let answer = match sent {
            Ok(()) => wait_for(reader, id, limit),
            Err(e) => Err((format!("the text reader would not take the request: {e}"), reused)),
        };
        let answer = match answer {
            Ok(answer) => answer,
            Err((why, retry)) => {
                // Late or gone: not a reader to ask again.
                let said = self.drop_reader(kept);
                let why = if said.is_empty() { why } else { format!("{why}: {said}") };
                return Err(Failed { why, retry: retry && reused });
            }
        };
        if let Some(answered) = answered.take() {
            answered();
        }
        if let Err(why) = self.moved_on(epoch) {
            // The answer is of a screen that was captured before the lock or Private mode.
            self.drop_reader(kept);
            return Err(why);
        }
        match answer["error"].as_str() {
            Some(why) => Err(failed(format!("the text reader could not read it: {why}"))),
            None => Ok(answer),
        }
    }

    /// End the running reader and say what it last said on its standard error.
    fn drop_reader(&self, kept: &mut Option<Kept>) -> String {
        *held(&self.child) = None;
        kept.take().map(end).unwrap_or_default()
    }

    /// End the reader, and with it everything it remembers of what was on screen. Returns at
    /// once, whatever a read is doing: a read in flight fails, and so does one that has not yet
    /// started its reader or has its answer in hand.
    pub fn forget(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        if let Some(child) = held(&self.child).take() {
            // Only the signal. Whoever holds `kept` collects it: the read in flight, or the
            // next read.
            let _ = held(&child).kill();
        }
    }
}

/// The answer with this `id`, from the reader's lines: anything else it said is dropped.
fn wait_for(reader: &Kept, id: u64, limit: Duration) -> Result<serde_json::Value, (String, bool)> {
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = reader.answers.recv_timeout(left).map_err(|why| match why {
            RecvTimeoutError::Timeout => (format!("the text reader did not finish in {} s", limit.as_secs()), false),
            RecvTimeoutError::Disconnected => ("the text reader stopped before it answered".to_string(), true),
        })?;
        match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(answer) if answer["id"].as_u64() == Some(id) => return Ok(answer),
            _ => {}
        }
    }
}

/// The epoch of the desktop's reader: take it when a read is asked for.
pub fn epoch() -> u64 {
    READER.epoch()
}

/// Ask the desktop's reader (the `yantrik-ocr` at `bin`) one thing, as a read that took `epoch`.
pub fn ask(bin: &Path, request: &serde_json::Value, limit: Duration, epoch: u64) -> Result<serde_json::Value, String> {
    READER.ask(|| serving(bin), request, limit, epoch)
}

/// End the desktop's reader: the screen locked, or the person went private, or they came back.
pub fn forget() {
    READER.forget();
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;
