//! The process: mpv spawned with its socket private, observed, and killed when dropped.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use super::client::{Client, Event};
use super::observed::Shared;

/// How long a fresh mpv gets to create its socket.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// The properties the window and the control surface read. Observed, so mpv pushes each change.
pub const OBSERVED: &[&str] = &["time-pos", "duration", "pause", "volume", "mute", "idle-active", "path"];

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

/// Where the next player's socket goes: the session's runtime directory (`runtime`, read from
/// `XDG_RUNTIME_DIR` at start), which must be ours and closed to everyone else, or there is no
/// safe place for it. Each player started gets a name of its own, so a player starting never
/// unlinks the socket of one still going away. Reads the disk: from a job only.
pub fn socket_path(runtime: Option<&Path>) -> Result<PathBuf, StartError> {
    use std::os::unix::fs::MetadataExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static STARTED: AtomicU64 = AtomicU64::new(0);
    let dir = runtime
        .ok_or_else(|| StartError::Failed("XDG_RUNTIME_DIR is not set, so there is no private place for its socket".into()))?;
    let meta = std::fs::symlink_metadata(dir)
        .map_err(|e| StartError::Failed(format!("{} cannot be read: {e}", dir.display())))?;
    // SAFETY: geteuid cannot fail and touches no memory.
    let me = unsafe { libc::geteuid() };
    if !meta.is_dir() || meta.uid() != me || meta.mode() & 0o077 != 0 {
        return Err(StartError::Failed(format!(
            "{} is not a directory only this account can open",
            dir.display()
        )));
    }
    let n = STARTED.fetch_add(1, Ordering::Relaxed) + 1;
    Ok(dir.join(format!("yantrik-music-mpv.{}.{n}.sock", std::process::id())))
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
    /// Called from a thread that lives as long as the app: `PR_SET_PDEATHSIG` fires when the
    /// thread that spawned the child ends. The app's job threads are that: the framework's pool
    /// keeps them until the process ends, and a job that panics is caught on its thread.
    pub fn start(
        program: &str,
        socket: PathBuf,
        volume: f64,
        forward: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Engine, StartError> {
        use std::os::unix::process::CommandExt;
        let forward = Arc::new(forward);
        // A player of an earlier process with this pid left it; it is ours, and mpv will not
        // bind over it.
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
        !matches!(self.child.lock().unwrap_or_else(|p| p.into_inner()).try_wait(), Ok(Some(_)))
    }

    /// The player's process id.
    pub fn pid(&self) -> u32 {
        self.child.lock().unwrap_or_else(|p| p.into_inner()).id()
    }

    /// Ends mpv and reaps it: asked first, so it closes its audio device cleanly, and killed if
    /// it has not gone in half a second. Its socket goes with it. Blocks for up to the reply
    /// timeout and that half second: from a job only. A second call finds it gone.
    pub fn stop(&self) {
        let mut child = self.child.lock().unwrap_or_else(|p| p.into_inner());
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = self.client.command(&[json!("quit")]);
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
        }
        let _ = child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

impl Drop for Engine {
    /// The last handle on a player that was never stopped (the process is ending with it still
    /// running) stops it here.
    fn drop(&mut self) {
        self.stop();
    }
}
