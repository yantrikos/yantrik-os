//! Private per-user directories for scratch files and kept state.
//!
//! Everything here exists because `/tmp` is shared. A fixed name there — a payload handed to
//! curl, a screenshot handed to a vision model, a task's output — is a name any other account on
//! the machine can create first: as a symlink, so our write lands on a file of their choosing, or
//! as a file of their own, so what we read back is theirs. Even a name we win the race for is
//! readable by everyone under the default umask, and these files hold page text, screenshots and
//! command output. So nothing of ours goes in `/tmp`, and there is deliberately no fallback to it:
//! when neither a runtime dir nor a home is known, the caller gets an error, not a shared path.
//!
//! A directory is only handed out after checking it is a real directory (not a link), owned by
//! the effective uid, and mode 0700 — whether we just made it or found it already there — and
//! that nothing on the way down to it from the runtime dir or home can be written by another
//! account. The checks themselves are in `check.rs`.

mod check;
mod upg;
#[cfg(all(test, unix))]
mod tests;

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use check::{current_uid, ensure_private_as, prepare, still_private};

pub use check::{create_private_file, open_private_file};

/// The scratch directory's name under `$XDG_RUNTIME_DIR`.
///
/// Not `yantrik`: that is the service socket directory (`socket_dir` in yantrik-ipc-transport),
/// holding companion.sock, vault.sock and the apps' single-instance pid files, which are opened
/// with a plain create that follows links. The file tools may write into scratch — it is where
/// they leave a diagram or a screenshot for the model to open again — so if the two were one
/// directory, a prompt-injected model could plant `notes.pid -> ~/.ssh/authorized_keys` there
/// and have the next app launch truncate it.
pub const SCRATCH_NAME: &str = "yantrik-scratch";

/// The work directory's name under `$XDG_RUNTIME_DIR`, and its fallback under `$HOME`.
///
/// Separate from scratch because scratch is one of the file tools' roots and this must not be:
/// other programs write here (whisper for seconds, edge-tts waiting on the network), following
/// links as they go, so nothing the model can reach may be able to reach in. The home fallback
/// sits under `$HOME`, which the tools can reach, so companion-core's `BLOCKED_SEGMENTS` names
/// both spellings.
pub const WORK_NAME: &str = "yantrik-work";
/// See [`WORK_NAME`].
pub const WORK_HOME_REL: &str = ".cache/yantrik/work";

/// The directory for short-lived files: `$XDG_RUNTIME_DIR/yantrik-scratch`, else
/// `$HOME/.cache/yantrik/tmp`. Canonical, so it compares cleanly against resolved paths.
///
/// The runtime dir comes first because it is what `/tmp` should have been for this: per user,
/// private, and emptied at logout.
///
/// The answer is remembered, because `validate_path` asks on every file-tool call; but it is
/// re-checked (still a 0700 directory of ours, not a link) each time before it is handed out, and
/// looked up afresh if it has gone — a runtime dir is emptied at logout, and a process can
/// outlive that.
pub fn scratch_dir() -> io::Result<PathBuf> {
    static CACHED: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut cached = CACHED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(dir) = cached.as_ref().filter(|dir| still_private(dir)) {
        return Ok(dir.clone());
    }
    let dir = scratch_dir_from(env_path("XDG_RUNTIME_DIR"), env_path("HOME"))?;
    *cached = Some(dir.clone());
    Ok(dir)
}

/// A named file inside [`scratch_dir`]. The name must be a plain file name, not a path.
pub fn scratch_file(name: &str) -> io::Result<PathBuf> {
    Ok(scratch_dir()?.join(plain_name(name)?))
}

/// [`scratch_file`] as a `String`, for the helpers and external commands that take a `&str`. A
/// path that is not UTF-8 is an error, not a lossy guess that names some other file.
pub fn scratch_file_string(name: &str) -> io::Result<String> {
    into_string(scratch_file(name)?)
}

/// Create (or empty) the named scratch file for writing, refusing to go through a link.
/// See [`create_private_file`] for what is refused and why.
pub fn create_scratch(name: &str) -> io::Result<File> {
    create_private_file(&scratch_file(name)?)
}

/// Open the named scratch file for reading, refusing a link, a FIFO, or a file that is not ours.
/// See [`open_private_file`]: a fixed name we read back is as plantable as one we write.
pub fn read_scratch(name: &str) -> io::Result<File> {
    open_private_file(&scratch_file(name)?)
}

/// The directory the per-call [`fresh_work_dir`]s are made in: `$XDG_RUNTIME_DIR/yantrik-work`,
/// else `$HOME/.cache/yantrik/work`. Private and checked like scratch, but out of the file tools'
/// reach (see [`WORK_NAME`]).
pub fn work_dir() -> io::Result<PathBuf> {
    work_dir_from(env_path("XDG_RUNTIME_DIR"), env_path("HOME"))
}

/// A new, empty, private directory inside [`work_dir`] for one call's files, removed (with
/// everything in it) when dropped.
///
/// For files another program must write — whisper's transcript, curl's download, ffmpeg's
/// output — where we cannot pass `O_NOFOLLOW` to its `open`. The name is random and made with a
/// plain `mkdir`, which fails if anything is already there, so nothing can be waiting inside it;
/// and it is not in scratch, so the model cannot list it and put something there while the
/// program runs — whisper takes seconds, edge-tts waits on the network.
pub fn fresh_work_dir(prefix: &str) -> io::Result<FreshDir> {
    use rand::Rng;
    let work = work_dir()?;
    let prefix = plain_name(prefix)?;
    let tag: u64 = rand::thread_rng().gen();
    let dir = work.join(format!("{prefix}-{tag:016x}"));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&dir)?;
    Ok(FreshDir(dir))
}

/// See [`fresh_work_dir`].
#[derive(Debug)]
pub struct FreshDir(PathBuf);

impl FreshDir {
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// A file inside, as the string a command line takes. The name must be a plain file name.
    pub fn file(&self, name: &str) -> io::Result<String> {
        into_string(self.0.join(plain_name(name)?))
    }
}

impl Drop for FreshDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write `contents` to the named scratch file through [`create_scratch`], and return its path as
/// the string a caller hands on (to the model, to a player).
pub fn write_scratch(name: &str, contents: &[u8]) -> io::Result<String> {
    use std::io::Write;
    let path = scratch_file_string(name)?;
    create_scratch(name)?.write_all(contents)?;
    Ok(path)
}

/// A private (0700) directory inside [`scratch_dir`], made if missing — for a run that needs a
/// whole directory of its own, such as a script's stand-in `HOME`.
pub fn scratch_subdir(name: &str) -> io::Result<PathBuf> {
    let dir = scratch_dir()?.join(plain_name(name)?);
    ensure_private_as(&dir, current_uid())?;
    Ok(dir)
}

/// A private directory for state that must outlive the session:
/// `$XDG_STATE_HOME/yantrik/<name>`, else `$HOME/.local/state/yantrik/<name>`.
pub fn state_dir(name: &str) -> io::Result<PathBuf> {
    state_dir_from(env_path("XDG_STATE_HOME"), env_path("HOME"), name)
}

// ── Resolution, with the environment passed in so tests need not mutate it ──────────────────

fn scratch_dir_from(runtime: Option<PathBuf>, home: Option<PathBuf>) -> io::Result<PathBuf> {
    resolve(runtime, home, SCRATCH_NAME, ".cache/yantrik/tmp")
}

fn work_dir_from(runtime: Option<PathBuf>, home: Option<PathBuf>) -> io::Result<PathBuf> {
    resolve(runtime, home, WORK_NAME, WORK_HOME_REL)
}

/// `runtime/runtime_name` if the runtime dir exists and passes, else `home/home_rel`.
fn resolve(runtime: Option<PathBuf>, home: Option<PathBuf>, runtime_name: &str, home_rel: &str) -> io::Result<PathBuf> {
    // The runtime dir itself is never created here: if it is set but missing (WSL without
    // systemd, a bare ssh session) the session has no runtime dir, and making one under /run is
    // neither possible for a user nor ours to do.
    if let Some(runtime) = runtime.filter(|r| r.is_dir()) {
        match prepare(&runtime, Path::new(runtime_name)) {
            Ok(dir) => return Ok(dir),
            Err(e) => tracing::warn!(
                runtime = %runtime.display(), error = %e,
                "runtime {runtime_name} dir refused; using the one under home"
            ),
        }
    }
    let home = home.ok_or_else(|| no_place("neither XDG_RUNTIME_DIR nor HOME is usable"))?;
    prepare(&home, Path::new(home_rel))
}

fn state_dir_from(state_home: Option<PathBuf>, home: Option<PathBuf>, name: &str) -> io::Result<PathBuf> {
    let name = plain_name(name)?;
    match (state_home, home) {
        (Some(state), _) => prepare(&state, &Path::new("yantrik").join(name)),
        (None, Some(home)) => prepare(&home, &Path::new(".local/state/yantrik").join(name)),
        (None, None) => Err(no_place("neither XDG_STATE_HOME nor HOME is set")),
    }
}

/// An environment path, only if it is absolute. The XDG spec says a relative value is to be
/// ignored, and a relative path would resolve against whatever the working directory is.
fn env_path(var: &str) -> Option<PathBuf> {
    absolute(std::env::var_os(var))
}

fn absolute(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|p| p.is_absolute())
}

fn plain_name(name: &str) -> io::Result<&str> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("not a plain file name: {name:?}")));
    }
    Ok(name)
}

fn into_string(path: PathBuf) -> io::Result<String> {
    path.into_os_string().into_string().map_err(|p| {
        io::Error::new(io::ErrorKind::InvalidData, format!("scratch path is not UTF-8: {}", p.to_string_lossy()))
    })
}

fn no_place(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("no private directory for yantrik files: {why}"))
}
