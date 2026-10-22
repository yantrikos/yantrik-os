//! The socket: a reader thread that sorts mpv's lines into replies and events, and a writer
//! that sends one command at a time.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// How long one command waits for mpv's reply. mpv answers a local socket in well under a
/// millisecond; this is for a player that is wedged, so the caller gets a sentence, not a hang.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// What mpv said, as the reader thread sorted it.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// An observed property changed. `Value::Null` when mpv has no value for it (no file).
    Property(String, Value),
    /// A file started playing.
    FileLoaded,
    /// A file stopped: `reason` is mpv's (`eof`, `stop`, `quit`, `error`, `redirect`), and
    /// `error` carries mpv's `file_error` when there is one.
    EndFile { reason: String, error: Option<String> },
    /// The socket closed: mpv exited or was killed.
    Gone,
}

type Reply = Result<Value, String>;
type Pending = Arc<Mutex<HashMap<u64, mpsc::Sender<Reply>>>>;

/// A connection to mpv's IPC socket.
pub struct Client {
    writer: Mutex<UnixStream>,
    pending: Pending,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
}

impl Client {
    /// Connect to the socket at `path` and start reading. `on_event` runs on the reader thread
    /// for every event, and once with [`Event::Gone`] when the socket closes.
    pub fn connect(path: &Path, on_event: impl Fn(Event) + Send + 'static) -> std::io::Result<Client> {
        let stream = UnixStream::connect(path)?;
        let reader = stream.try_clone()?;
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        {
            let pending = pending.clone();
            let alive = alive.clone();
            std::thread::Builder::new().name("mpv-ipc".into()).spawn(move || {
                for line in BufReader::new(reader).lines() {
                    let Ok(line) = line else { break };
                    match parse_line(&line) {
                        Some(Line::Reply(id, reply)) => {
                            if let Some(waiter) = pending.lock().unwrap().remove(&id) {
                                let _ = waiter.send(reply);
                            }
                        }
                        Some(Line::Event(event)) => on_event(event),
                        None => {}
                    }
                }
                alive.store(false, Ordering::SeqCst);
                // Everyone still waiting is told now rather than at their timeout.
                for (_, waiter) in pending.lock().unwrap().drain() {
                    let _ = waiter.send(Err("the player (mpv) stopped".into()));
                }
                on_event(Event::Gone);
            })?;
        }
        Ok(Client { writer: Mutex::new(stream), pending, next_id: AtomicU64::new(1), alive })
    }

    /// Whether the socket is still open.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Run one mpv command and return its `data`, or mpv's error in its own words.
    ///
    /// `args` is the command as mpv's JSON IPC takes it: the name, then each argument as its own
    /// element. Nothing here is ever joined into a string or handed to a shell.
    pub fn command(&self, args: &[Value]) -> Reply {
        if !self.is_alive() {
            return Err("the player (mpv) is not running".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let mut line = json!({ "command": args, "request_id": id }).to_string();
        line.push('\n');
        let written = self.writer.lock().unwrap().write_all(line.as_bytes());
        if let Err(e) = written {
            self.pending.lock().unwrap().remove(&id);
            return Err(format!("could not reach the player (mpv): {e}"));
        }
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(reply) => reply,
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err("the player (mpv) did not answer".into())
            }
        }
    }

    pub fn get(&self, property: &str) -> Reply {
        self.command(&[json!("get_property"), json!(property)])
    }

    pub fn set(&self, property: &str, value: Value) -> Result<(), String> {
        self.command(&[json!("set_property"), json!(property), value]).map(|_| ())
    }

    /// Queue `path` to play now, replacing whatever was playing. The path is one JSON string.
    pub fn load(&self, path: &str) -> Result<(), String> {
        self.command(&[json!("loadfile"), json!(path), json!("replace")]).map(|_| ())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Ends the reader thread too: its half of the socket sees EOF.
        if let Ok(w) = self.writer.lock() {
            let _ = w.shutdown(std::net::Shutdown::Both);
        }
    }
}

enum Line {
    Reply(u64, Reply),
    Event(Event),
}

/// Sort one line from mpv. `None` for anything that is neither a reply we asked for nor an
/// event we use.
fn parse_line(line: &str) -> Option<Line> {
    let v: Value = serde_json::from_str(line).ok()?;
    if let Some(event) = v.get("event").and_then(Value::as_str) {
        return match event {
            "property-change" => Some(Line::Event(Event::Property(
                v.get("name")?.as_str()?.to_string(),
                v.get("data").cloned().unwrap_or(Value::Null),
            ))),
            "file-loaded" => Some(Line::Event(Event::FileLoaded)),
            "end-file" => Some(Line::Event(Event::EndFile {
                reason: v.get("reason").and_then(Value::as_str).unwrap_or("").to_string(),
                error: v.get("file_error").and_then(Value::as_str).map(str::to_string),
            })),
            _ => None,
        };
    }
    let id = v.get("request_id")?.as_u64()?;
    let error = v.get("error").and_then(Value::as_str).unwrap_or("");
    Some(Line::Reply(
        id,
        if error == "success" {
            Ok(v.get("data").cloned().unwrap_or(Value::Null))
        } else {
            Err(format!("mpv said: {error}"))
        },
    ))
}
