//! One headless mpv, driven over its JSON IPC socket.
//!
//! The app starts one `mpv --idle` per instance, with no window, no terminal, no config and no
//! scripts, and talks to it over a Unix socket in `$XDG_RUNTIME_DIR` that only this account can
//! open (mode 0600). Every command is a JSON array whose elements are mpv's own arguments, so a
//! file path travels as one string to `loadfile` and never through a shell.
//!
//! Two halves:
//!
//! - [`Client`] is the socket: a reader thread that sorts mpv's lines into replies (matched to the
//!   waiting command by `request_id`) and events, and a writer that sends one command at a time.
//!   It knows nothing about processes, which is what lets the tests drive it against a fake
//!   socket.
//! - [`Engine`] is the process: it spawns mpv, waits for the socket, observes the properties the
//!   window draws, keeps the last value mpv reported for each in [`Observed`], and kills mpv when
//!   it is dropped. `PR_SET_PDEATHSIG` covers the case where the app dies without dropping it.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// How long one command waits for mpv's reply. mpv answers a local socket in well under a
/// millisecond; this is for a player that is wedged, so the caller gets a sentence, not a hang.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a fresh mpv gets to create its socket.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// The properties the window and the control surface read. Observed, so mpv pushes each change.
pub const OBSERVED: &[&str] = &["time-pos", "duration", "pause", "volume", "mute", "idle-active", "path"];

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

// ── What mpv last reported ──────────────────────────────────────────

/// The last value mpv reported for every observed property, and the last file that ended.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Observed {
    pub time_pos: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    pub volume: Option<f64>,
    pub muted: bool,
    pub idle: bool,
    pub path: Option<String>,
    /// How many files have loaded since the engine started; a waiter compares it.
    pub loads: u64,
    /// The last `end-file` with an error: what mpv said.
    pub last_error: Option<String>,
    /// How many files have failed since the engine started; a waiter compares it, so an old
    /// failure is never read as the answer to a new load.
    pub errors: u64,
    pub gone: bool,
}

impl Observed {
    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::Property(name, v) => match name.as_str() {
                "time-pos" => self.time_pos = v.as_f64(),
                "duration" => self.duration = v.as_f64(),
                "pause" => self.paused = v.as_bool().unwrap_or(false),
                "volume" => self.volume = v.as_f64(),
                "mute" => self.muted = v.as_bool().unwrap_or(false),
                "idle-active" => self.idle = v.as_bool().unwrap_or(false),
                "path" => self.path = v.as_str().map(str::to_string),
                _ => {}
            },
            Event::FileLoaded => {
                self.loads += 1;
                self.last_error = None;
            }
            Event::EndFile { reason, error } => match reason.as_str() {
                "error" => {
                    self.errors += 1;
                    self.last_error = Some(error.clone().unwrap_or_else(|| "it could not be played".into()));
                }
                // The file turned out to be a playlist and mpv went on to what it lists. That is
                // a refusal: what was asked for did not play.
                "redirect" => {
                    self.errors += 1;
                    self.last_error = Some(REDIRECTED.into());
                }
                _ => {}
            },
            Event::Gone => self.gone = true,
        }
    }

    /// Playing means a file is loaded and not paused, by mpv's account.
    pub fn playing(&self) -> bool {
        !self.gone && !self.idle && self.path.is_some() && !self.paused
    }
}

/// [`Observed`] behind a lock, with a condition to wait on for the next change.
#[derive(Default)]
pub struct Shared {
    state: Mutex<Observed>,
    changed: Condvar,
}

impl Shared {
    pub fn apply(&self, event: &Event) {
        self.state.lock().unwrap().apply(event);
        self.changed.notify_all();
    }

    pub fn snapshot(&self) -> Observed {
        self.state.lock().unwrap().clone()
    }

    /// Wait up to `limit` for `done` to hold of what mpv has reported, and return the last
    /// report either way.
    pub fn wait_for(&self, limit: Duration, done: impl Fn(&Observed) -> bool) -> (bool, Observed) {
        let deadline = Instant::now() + limit;
        let mut state = self.state.lock().unwrap();
        loop {
            if done(&state) {
                return (true, state.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                return (false, state.clone());
            }
            state = self.changed.wait_timeout(state, deadline - now).unwrap().0;
        }
    }
}

// ── The process ─────────────────────────────────────────────────────

/// Why mpv could not be started, in words for the person.
#[derive(Debug, Clone, PartialEq)]
pub enum StartError {
    /// No `mpv` on the PATH.
    Missing,
    Failed(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::Missing => f.write_str(
                "Music cannot play: the player it uses, mpv, is not installed on this machine.",
            ),
            StartError::Failed(why) => write!(f, "Music cannot play: the player (mpv) did not start — {why}."),
        }
    }
}

/// One running mpv, its socket and what it last reported.
pub struct Engine {
    pub client: Client,
    pub observed: Arc<Shared>,
    child: Mutex<Child>,
    socket: PathBuf,
}

/// Where this instance's socket goes: the session's runtime directory, which must be ours and
/// closed to everyone else, or there is no safe place for it.
pub fn socket_path() -> Result<PathBuf, StartError> {
    use std::os::unix::fs::MetadataExt;
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| StartError::Failed("XDG_RUNTIME_DIR is not set, so there is no private place for its socket".into()))?;
    let meta = std::fs::symlink_metadata(&dir)
        .map_err(|e| StartError::Failed(format!("{} cannot be read: {e}", dir.display())))?;
    // SAFETY: geteuid cannot fail and touches no memory.
    let me = unsafe { libc::geteuid() };
    if !meta.is_dir() || meta.uid() != me || meta.mode() & 0o077 != 0 {
        return Err(StartError::Failed(format!(
            "{} is not a directory only this account can open",
            dir.display()
        )));
    }
    Ok(dir.join(format!("yantrik-music-mpv.{}.sock", std::process::id())))
}

/// ffmpeg opens nothing but local files, and only reads the containers Music's sound files come in.
/// (`[...]` keeps the commas inside one value for mpv.)
const LAVF_LOCAL_ONLY: &str = "--demuxer-lavf-o=protocol_whitelist=file,format_whitelist=[wav,mp3,flac,ogg,mov,aac]";

/// mpv's command line, apart from the program.
///
/// mpv decides what a file is from its contents, not its name, so a file called `x.mp3` that holds
/// an m3u playlist would otherwise be opened as one, and mpv would then fetch every URL and open
/// every file listed in it: past `play_file`'s rule, and onto the network. So:
///
/// - `--access-references=no`: a file never opens other files or addresses it names (playlists,
///   EDL, ordered chapters). Such a file fails with "unrecognized file format".
/// - `--load-unsafe-playlists=no`: playlists from untrusted places stay refused even if a demuxer
///   produces one anyway. (mpv's default, set so a config change cannot turn it on.)
/// - `--demuxer-lavf-o=protocol_whitelist=file,...`: ffmpeg also picks formats by content, and
///   some of them (SDP, RTSP, HLS, DASH, concat...) open sockets or URLs themselves: an `x.mp3`
///   holding an SDP description would bind UDP ports on every interface and play what arrives.
///   `protocol_whitelist=file` lets nothing ffmpeg opens on its own be anything but a local file,
///   whatever the format; `format_whitelist` keeps ffmpeg to the containers of the sound files
///   Music lists (wav, mp3, flac, ogg/opus, m4a), so any other format is refused outright.
/// - `--ytdl=no`: no youtube-dl for anything that looks like a web page.
/// - `--network-timeout=5`: if anything ever does reach the network, it gives up in seconds
///   instead of holding the player.
pub fn args(volume: f64, socket: &Path) -> Vec<String> {
    let mut args: Vec<String> = [
        "--idle=yes",
        "--no-config",
        "--load-scripts=no",
        "--ytdl=no",
        "--access-references=no",
        "--load-unsafe-playlists=no",
        LAVF_LOCAL_ONLY,
        "--network-timeout=5",
        "--no-video",
        "--audio-display=no",
        "--force-window=no",
        "--no-terminal",
        "--input-default-bindings=no",
        "--keep-open=no",
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    args.push(format!("--volume={}", volume.clamp(0.0, 100.0).round()));
    args.push(format!("--input-ipc-server={}", socket.display()));
    args
}

/// What the caller hears when mpv found a playlist where a sound file should be.
pub const REDIRECTED: &str =
    "it holds a playlist that points at other files or addresses, not sound, and Music does not follow those";

impl Engine {
    /// Start mpv with its socket at `socket` and observe [`OBSERVED`]. `forward` gets every
    /// event after [`Observed`] has taken it in. `volume` is the starting volume, 0–100.
    ///
    /// Must be called from the main thread: `PR_SET_PDEATHSIG` fires when the thread that
    /// spawned the child ends, and only the main thread lives as long as the app.
    pub fn start(
        program: &str,
        socket: PathBuf,
        volume: f64,
        forward: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Engine, StartError> {
        use std::os::unix::process::CommandExt;
        let forward = Arc::new(forward);
        // An earlier instance with this pid left it; it is ours, and mpv will not bind over it.
        let _ = std::fs::remove_file(&socket);
        let mut command = Command::new(program);
        command
            .args(args(volume, &socket))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: getpid cannot fail and touches no memory.
        let parent = unsafe { libc::getpid() };
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            command.pre_exec(move || {
                // The socket mpv creates is born 0600: no window in which another account could
                // connect before the chmod below.
                libc::umask(0o177);
                // mpv exits with the app even when the app is killed and never drops the engine.
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                // The app may have died between the fork and the prctl, before there was anyone
                // to signal us: then nobody would ever stop this mpv, so it does not start.
                if libc::getppid() != parent {
                    libc::_exit(1);
                }
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StartError::Missing,
            _ => StartError::Failed(e.to_string()),
        })?;

        let observed = Arc::new(Shared::default());
        let deadline = Instant::now() + START_TIMEOUT;
        let client = loop {
            if let Ok(Some(status)) = child.try_wait() {
                let _ = std::fs::remove_file(&socket);
                return Err(StartError::Failed(format!("it exited at once ({status})")));
            }
            if socket.exists() {
                let sink = observed.clone();
                let forward = forward.clone();
                let on_event = move |event: Event| {
                    sink.apply(&event);
                    forward(event);
                };
                match Client::connect(&socket, on_event) {
                    Ok(c) => break c,
                    // The file can exist a moment before mpv listens on it; try again.
                    Err(e) if Instant::now() >= deadline => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(StartError::Failed(format!("its socket would not open: {e}")));
                    }
                    Err(_) => {}
                }
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(StartError::Failed("it never opened its control socket".into()));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // Belt and braces over the umask: whatever mpv did, only this account opens the socket.
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600));
        }
        let engine = Engine { client, observed, child: Mutex::new(child), socket };
        for (n, property) in OBSERVED.iter().enumerate() {
            engine
                .client
                .command(&[json!("observe_property"), json!(n as u64 + 1), json!(property)])
                .map_err(|e| StartError::Failed(format!("it would not report `{property}`: {e}")))?;
        }
        Ok(engine)
    }

    pub fn is_alive(&self) -> bool {
        if !self.client.is_alive() {
            return false;
        }
        !matches!(self.child.lock().unwrap().try_wait(), Ok(Some(_)))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Asked first, so mpv closes its audio device cleanly; killed if it does not go.
        let _ = self.client.command(&[json!("quit")]);
        let mut child = self.child.lock().unwrap();
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Ok(None) = child.try_wait() {
            let _ = child.kill();
        }
        let _ = child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixListener;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-music-mpv-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stand-in for mpv: answers every command with `answer(command)`, first sending the events
    /// `before(command)` returns, and records what it was sent.
    fn fake_mpv(
        socket: &Path,
        answer: impl Fn(&Value) -> Value + Send + 'static,
        before: impl Fn(&Value) -> Vec<Value> + Send + 'static,
    ) -> Arc<Mutex<Vec<Value>>> {
        let listener = UnixListener::bind(socket).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut out = stream.try_clone().unwrap();
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                let v: Value = serde_json::from_str(&line).unwrap();
                log.lock().unwrap().push(v.clone());
                for event in before(&v["command"]) {
                    writeln!(out, "{event}").unwrap();
                }
                let mut reply = answer(&v["command"]);
                reply["request_id"] = v["request_id"].clone();
                writeln!(out, "{reply}").unwrap();
                if v["command"][0] == "quit" {
                    break;
                }
            }
        });
        seen
    }

    #[test]
    fn a_command_is_sent_as_a_json_array_and_answered_by_its_request_id() {
        let dir = scratch("cmd");
        let socket = dir.join("mpv.sock");
        let seen = fake_mpv(
            &socket,
            |c| match c[0].as_str() {
                Some("get_property") if c[1] == "volume" => json!({"error": "success", "data": 64.0}),
                Some("get_property") => json!({"error": "property unavailable"}),
                _ => json!({"error": "success"}),
            },
            |_| vec![],
        );
        let client = Client::connect(&socket, |_| {}).unwrap();

        assert_eq!(client.get("volume"), Ok(json!(64.0)));
        assert_eq!(client.get("duration"), Err("mpv said: property unavailable".into()));

        // A name a shell would split, expand or run is one argument, untouched.
        let nasty = "/home/me/Music/$(rm -rf ~); `id` 'quoted' \"and\" spaced.mp3";
        client.load(nasty).unwrap();
        let sent = seen.lock().unwrap().clone();
        let load = sent.iter().find(|v| v["command"][0] == "loadfile").expect("loadfile was sent");
        assert_eq!(load["command"], json!(["loadfile", nasty, "replace"]));
        assert!(load["request_id"].as_u64().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn events_are_sorted_from_replies_and_kept_as_what_mpv_reported() {
        let dir = scratch("events");
        let socket = dir.join("mpv.sock");
        fake_mpv(
            &socket,
            |_| json!({"error": "success"}),
            |c| {
                if c[0] == "loadfile" {
                    vec![
                        json!({"event": "property-change", "id": 7, "name": "path", "data": c[1]}),
                        json!({"event": "file-loaded"}),
                        json!({"event": "property-change", "id": 2, "name": "duration", "data": 245.5}),
                        json!({"event": "property-change", "id": 1, "name": "time-pos", "data": 0.25}),
                        json!({"event": "property-change", "id": 6, "name": "idle-active", "data": false}),
                        json!({"event": "end-file", "reason": "eof"}),
                    ]
                } else {
                    vec![]
                }
            },
        );
        let shared = Arc::new(Shared::default());
        let (tx, rx) = mpsc::channel();
        let sink = shared.clone();
        let tx = Mutex::new(tx);
        let client = Client::connect(&socket, move |e| {
            sink.apply(&e);
            let _ = tx.lock().unwrap().send(e);
        })
        .unwrap();
        client.load("/music/a.flac").unwrap();
        let (ok, seen) = shared.wait_for(Duration::from_secs(2), |o| o.time_pos.is_some());
        assert!(ok, "{seen:?}");
        assert_eq!(seen.path.as_deref(), Some("/music/a.flac"));
        assert_eq!(seen.duration, Some(245.5));
        assert_eq!(seen.loads, 1);
        assert!(seen.playing());
        let events: Vec<Event> = rx.try_iter().collect();
        assert!(events.contains(&Event::EndFile { reason: "eof".into(), error: None }), "{events:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_player_that_goes_away_is_said_once_and_fails_waiting_commands() {
        let dir = scratch("gone");
        let socket = dir.join("mpv.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Read one command, then die without answering it.
            let mut buf = [0u8; 64];
            let _ = stream.read(&mut buf);
        });
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let client = Client::connect(&socket, move |e| {
            let _ = tx.lock().unwrap().send(e);
        })
        .unwrap();
        let reply = client.get("pause");
        assert_eq!(reply, Err("the player (mpv) stopped".into()));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Event::Gone));
        assert!(!client.is_alive());
        assert_eq!(client.get("pause"), Err("the player (mpv) is not running".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_player_is_said_in_plain_words() {
        let dir = scratch("missing");
        let err = Engine::start("/nonexistent/mpv-is-not-here", dir.join("s.sock"), 80.0, |_| {})
            .err()
            .expect("no such program");
        assert_eq!(err, StartError::Missing);
        assert!(err.to_string().contains("mpv, is not installed"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_error_at_the_end_of_a_file_is_kept_until_the_next_one_loads() {
        let mut o = Observed::default();
        o.apply(&Event::EndFile { reason: "error".into(), error: Some("unrecognized file format".into()) });
        assert_eq!(o.last_error.as_deref(), Some("unrecognized file format"));
        o.apply(&Event::FileLoaded);
        assert_eq!(o.last_error, None);
        o.apply(&Event::Gone);
        assert!(!o.playing());
    }

    #[test]
    fn a_playlist_where_a_sound_file_should_be_counts_as_a_failure() {
        let mut o = Observed::default();
        o.apply(&Event::EndFile { reason: "redirect".into(), error: None });
        assert_eq!(o.errors, 1);
        assert_eq!(o.last_error.as_deref(), Some(REDIRECTED));
    }

    #[test]
    fn mpv_is_told_never_to_follow_what_a_file_points_at() {
        let args = args(150.0, Path::new("/run/user/1000/m.sock"));
        for flag in [
            "--access-references=no",
            "--load-unsafe-playlists=no",
            "--ytdl=no",
            "--no-config",
            "--load-scripts=no",
            "--demuxer-lavf-o=protocol_whitelist=file,format_whitelist=[wav,mp3,flac,ogg,mov,aac]",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag} missing from {args:?}");
        }
        assert!(args.iter().any(|a| a.starts_with("--network-timeout=")), "{args:?}");
        assert!(args.contains(&"--volume=100".to_string()), "{args:?}");
        assert_eq!(args.last().unwrap(), "--input-ipc-server=/run/user/1000/m.sock");
    }

    /// The real thing, where the machine has it: the socket is 0600, a file plays, and mpv is gone
    /// once the engine is dropped.
    #[test]
    fn a_real_mpv_runs_behind_a_private_socket_and_exits_with_the_engine() {
        if Command::new("mpv").arg("--version").stdout(Stdio::null()).status().is_err() {
            eprintln!("no mpv here; skipped");
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("real");
        let socket = dir.join("mpv.sock");
        let engine = Engine::start("mpv", socket.clone(), 0.0, |_| {}).expect("mpv starts");
        let mode = std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the socket is this account's alone");
        assert_eq!(engine.client.get("volume"), Ok(json!(0.0)));
        let pid = engine.child.lock().unwrap().id();
        drop(engine);
        assert!(!socket.exists(), "the socket goes with the engine");
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        assert!(state.is_empty() || state.contains(") Z"), "mpv is gone: {state}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A short, silent WAV that mpv plays.
    fn wav(path: &Path) {
        let (rate, samples) = (8000u32, 4000u32);
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + samples * 2).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(samples * 2).to_le_bytes());
        b.resize(b.len() + samples as usize * 2, 0);
        std::fs::write(path, b).unwrap();
    }

    /// The security review's file: `evil.mp3` is an m3u playlist naming a URL and a file. A real
    /// mpv refuses it without contacting the listener or opening the file, and a real sound file
    /// after it still plays.
    #[test]
    fn a_real_mpv_does_not_follow_a_playlist_named_like_a_sound_file() {
        if Command::new("mpv").arg("--version").stdout(Stdio::null()).status().is_err() {
            eprintln!("no mpv here; skipped");
            return;
        }
        let dir = scratch("evil");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let outside = dir.join("outside.wav");
        wav(&outside);
        let evil = dir.join("evil.mp3");
        std::fs::write(&evil, format!("#EXTM3U\nhttp://127.0.0.1:{port}/beacon.mp3\n{}\n", outside.display())).unwrap();
        let good = dir.join("good.wav");
        wav(&good);

        let engine = Engine::start("mpv", dir.join("mpv.sock"), 0.0, |_| {}).expect("mpv starts");
        engine.client.load(evil.to_str().unwrap()).unwrap();
        let (ended, o) = engine.observed.wait_for(Duration::from_secs(5), |o| o.errors > 0 || o.loads > 0);
        assert!(ended, "mpv said something about evil.mp3: {o:?}");
        assert_eq!(o.loads, 0, "nothing played, neither the URL nor outside.wav: {o:?}");
        assert_ne!(o.last_error.as_deref(), Some(REDIRECTED), "mpv did not even open it as a playlist");
        // Give a fetch that slipped through time to arrive.
        std::thread::sleep(Duration::from_millis(500));
        match listener.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("mpv contacted the address in the playlist: {other:?}"),
        }

        engine.client.load(good.to_str().unwrap()).unwrap();
        let want = good.to_str().unwrap().to_string();
        let (played, o) =
            engine.observed.wait_for(Duration::from_secs(5), |o| o.loads > 0 && o.path.as_deref() == Some(want.as_str()));
        assert!(played, "a real sound file still plays: {o:?}");
        drop(engine);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The security delta review's file: `sdp.mp3` is an SDP description, which ffmpeg would open
    /// as a live RTP stream on UDP ports of every interface. A real mpv refuses it, binds neither
    /// port, and a real sound file after it still plays.
    #[test]
    fn a_real_mpv_does_not_open_network_ports_for_an_sdp_file_named_like_a_sound_file() {
        if Command::new("mpv").arg("--version").stdout(Stdio::null()).status().is_err() {
            eprintln!("no mpv here; skipped");
            return;
        }
        use std::net::UdpSocket;
        let dir = scratch("sdp");
        // An even port whose next one is free too: RTP takes the port, RTCP the one after it.
        let port = (0..50)
            .find_map(|_| {
                let p = UdpSocket::bind("0.0.0.0:0").ok()?.local_addr().ok()?.port() & !1;
                (p > 1024 && UdpSocket::bind(("0.0.0.0", p)).is_ok() && UdpSocket::bind(("0.0.0.0", p + 1)).is_ok())
                    .then_some(p)
            })
            .expect("a free pair of UDP ports");
        let sdp = dir.join("sdp.mp3");
        std::fs::write(&sdp, format!("v=0\no=- 0 0 IN IP4 127.0.0.1\ns=x\nc=IN IP4 127.0.0.1\nt=0 0\nm=audio {port} RTP/AVP 0\n"))
            .unwrap();
        let good = dir.join("good.wav");
        wav(&good);

        let engine = Engine::start("mpv", dir.join("mpv.sock"), 0.0, |_| {}).expect("mpv starts");
        engine.client.load(sdp.to_str().unwrap()).unwrap();
        let (ended, o) = engine.observed.wait_for(Duration::from_secs(5), |o| o.errors > 0 || o.loads > 0);
        // Checked while this mpv still runs: a socket it bound would still be held.
        for p in [port, port + 1] {
            assert!(UdpSocket::bind(("0.0.0.0", p)).is_ok(), "mpv bound UDP port {p} for sdp.mp3: {o:?}");
        }
        assert!(ended, "mpv refused sdp.mp3 instead of waiting on the network: {o:?}");
        assert_eq!(o.loads, 0, "nothing played: {o:?}");

        engine.client.load(good.to_str().unwrap()).unwrap();
        let want = good.to_str().unwrap().to_string();
        let (played, o) =
            engine.observed.wait_for(Duration::from_secs(5), |o| o.loads > 0 && o.path.as_deref() == Some(want.as_str()));
        assert!(played, "a real sound file still plays: {o:?}");
        drop(engine);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
