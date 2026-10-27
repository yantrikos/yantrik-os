//! Starting an app of the framework on demand, with no window, for an agent (design #978 step 1).
//!
//! An agent's act or describe on an app that is not running used to be told to open it, and a Mind
//! that opened it got a window of its own. Now the shell starts the app's one instance itself:
//! on the person's display, from the person's session, with `--service`, so it loads its store and
//! serves intents and `describe` and shows nothing. This module is the part that needs no window
//! system: what to do when asked ([`Starts::plan`]), what a start that ended means
//! ([`Starts::exited`]), and the command that starts one ([`service_command`]).
//!
//! The shell asks the instance's socket whether it answers; a start that is under way is not
//! started again, so two agents asking at once start one instance, and one that failed is
//! remembered for [`FAILED_FOR`], so the second ask of a caller waiting out a start hears why.

use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};

/// How long a start that has not answered is still believed to be under way.
pub const STARTING_FOR: Duration = Duration::from_secs(30);

/// How long a failed start is remembered before the next ask starts again.
pub const FAILED_FOR: Duration = Duration::from_secs(10);

/// What the shell does about an ask to start `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// The instance answers: nothing to start.
    Running,
    /// A start is under way: nothing more to start.
    Starting,
    /// The last start failed, and why.
    Failed(String),
    /// Start the instance now.
    Spawn,
}

#[derive(Debug, Clone)]
enum State {
    Starting { since: Instant },
    Failed { why: String, at: Instant },
}

/// The starts the shell has made, by app id.
#[derive(Debug, Default)]
pub struct Starts {
    by_id: HashMap<String, State>,
}

impl Starts {
    /// What to do about an ask to start `id`, `answering` being whether its surface answers now.
    /// A [`Plan::Spawn`] is recorded as under way before it is returned, so the next ask in the
    /// same instant is [`Plan::Starting`].
    pub fn plan(&mut self, id: &str, answering: bool, now: Instant) -> Plan {
        if answering {
            self.by_id.remove(id);
            return Plan::Running;
        }
        match self.by_id.get(id) {
            Some(State::Starting { since }) if now.duration_since(*since) < STARTING_FOR => return Plan::Starting,
            Some(State::Failed { why, at }) if now.duration_since(*at) < FAILED_FOR => {
                return Plan::Failed(why.clone());
            }
            _ => {}
        }
        self.by_id.insert(id.to_string(), State::Starting { since: now });
        Plan::Spawn
    }

    /// The process the shell started for `id` ended. `answering` is whether the surface answers
    /// now, and `how` says how it ended for a person to read. An instance that answers is a start
    /// that worked, whoever holds it. One that ended cleanly without answering has most likely
    /// found another copy starting, and is still believed for [`STARTING_FOR`]; one that ended in
    /// failure is a failed start.
    pub fn exited(&mut self, id: &str, success: bool, answering: bool, how: &str, now: Instant) {
        if answering {
            self.by_id.remove(id);
        } else if !success {
            self.by_id.insert(id.to_string(), State::Failed { why: how.to_string(), at: now });
        }
    }

    /// The process could not be started at all.
    pub fn not_spawned(&mut self, id: &str, why: &str, now: Instant) {
        self.by_id.insert(id.to_string(), State::Failed { why: why.to_string(), at: now });
    }
}

/// Whether a process listens at the socket `path`: a connection is tried without blocking, and
/// closed at once. A listener whose backlog is full is alive (and wedged), which is an answer here;
/// the shell's UI thread must never wait on it (#978 L8). A socket file nobody listens at, a stale
/// one left by a kill, is not.
#[cfg(unix)]
pub fn answers(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: a zeroed sockaddr_un is valid; the path is copied within its bounds with the final NUL
    // kept; the descriptor made here is closed here on every path.
    unsafe {
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        if bytes.is_empty() || bytes.len() >= addr.sun_path.len() {
            return false;
        }
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (to, from) in addr.sun_path.iter_mut().zip(bytes) {
            *to = *from as libc::c_char;
        }
        let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return false;
        }
        let rc = libc::connect(
            fd,
            &addr as *const libc::sockaddr_un as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        );
        let errno = if rc == 0 { 0 } else { std::io::Error::last_os_error().raw_os_error().unwrap_or(0) };
        libc::close(fd);
        rc == 0 || errno == libc::EAGAIN || errno == libc::EINPROGRESS
    }
}

#[cfg(not(unix))]
pub fn answers(_path: &std::path::Path) -> bool {
    false
}

/// The command that starts the instance of the app that runs `program`, with no window. It is
/// the shell's own to run, in the person's session: the caller that asked is not in it. Nothing
/// that marks a launch as a Mind's is set (an instance is the person's, on their display), a
/// fullscreen request from the shell's own start is dropped, and the flag is the only argument:
/// there is nothing to open.
pub fn service_command(program: &std::path::Path) -> Command {
    let mut command = Command::new(program);
    command
        .arg("--service")
        .env_remove("YANTRIK_FOR_MIND")
        .env_remove("SLINT_FULLSCREEN")
        .stdin(std::process::Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64, base: Instant) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn an_ask_starts_the_instance_once_and_a_second_ask_waits_for_it() {
        let base = Instant::now();
        let mut starts = Starts::default();
        assert_eq!(starts.plan("notes", false, base), Plan::Spawn);
        assert_eq!(starts.plan("notes", false, at(1, base)), Plan::Starting, "two agents at once");
        assert_eq!(starts.plan("calendar", false, at(1, base)), Plan::Spawn, "another app is its own");
        assert_eq!(starts.plan("notes", true, at(2, base)), Plan::Running);
        assert_eq!(starts.plan("notes", false, at(3, base)), Plan::Spawn, "it was killed: start again");
    }

    #[test]
    fn a_start_that_never_answers_is_believed_for_a_while_and_then_tried_again() {
        let base = Instant::now();
        let mut starts = Starts::default();
        assert_eq!(starts.plan("notes", false, base), Plan::Spawn);
        assert_eq!(starts.plan("notes", false, at(29, base)), Plan::Starting);
        assert_eq!(starts.plan("notes", false, at(31, base)), Plan::Spawn);
    }

    #[test]
    fn a_failed_start_is_told_to_the_next_ask_and_then_tried_again() {
        let base = Instant::now();
        let mut starts = Starts::default();
        assert_eq!(starts.plan("notes", false, base), Plan::Spawn);
        starts.exited("notes", false, false, "it exited with status 1", at(2, base));
        assert_eq!(starts.plan("notes", false, at(3, base)), Plan::Failed("it exited with status 1".into()));
        assert_eq!(starts.plan("notes", false, at(13, base)), Plan::Spawn, "after the wait");
    }

    #[test]
    fn a_clean_exit_that_does_not_answer_is_another_copy_starting_not_a_failure() {
        let base = Instant::now();
        let mut starts = Starts::default();
        assert_eq!(starts.plan("notes", false, base), Plan::Spawn);
        starts.exited("notes", true, false, "it ended", at(1, base));
        assert_eq!(starts.plan("notes", false, at(2, base)), Plan::Starting);
        starts.exited("notes", false, true, "it exited", at(3, base));
        assert_eq!(starts.plan("notes", false, at(4, base)), Plan::Spawn, "answering cleared it");
    }

    #[test]
    fn a_process_that_could_not_be_started_is_a_failed_start() {
        let base = Instant::now();
        let mut starts = Starts::default();
        starts.not_spawned("notes", "no such file", base);
        assert_eq!(starts.plan("notes", false, at(1, base)), Plan::Failed("no such file".into()));
    }

    /// The instance is the person's, started by the shell: no mark of a Mind's launch, no
    /// fullscreen, nothing to open, and no input.
    #[test]
    fn the_command_is_the_flag_alone_with_the_marks_of_a_minds_launch_removed() {
        let command = service_command(std::path::Path::new("/opt/yantrik/bin/yantrik-notes"));
        let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, vec!["--service"]);
        let envs: Vec<_> = command.get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.is_none())).collect();
        assert!(envs.contains(&("YANTRIK_FOR_MIND".to_string(), true)), "{envs:?}");
        assert!(envs.contains(&("SLINT_FULLSCREEN".to_string(), true)), "{envs:?}");
    }

    /// L8: whether a socket is listened at is asked without blocking.
    #[cfg(unix)]
    #[test]
    fn a_listener_answers_and_a_stale_or_missing_socket_does_not() {
        let dir = std::env::temp_dir().join(format!("yantrik-answers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app-notes.sock");
        assert!(!answers(&path), "no file");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(answers(&path), "a listener");
        drop(listener);
        assert!(path.exists() && !answers(&path), "a socket file a killed process left behind");
        assert!(!answers(&dir.join("x".repeat(200))), "a path too long to be a socket");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
