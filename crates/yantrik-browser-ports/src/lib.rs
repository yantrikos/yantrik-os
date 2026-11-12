//! Which profile each of the desktop's browsers uses, and how to tell which browser holds a port
//! (#1085).
//!
//! There are two browsers and they never share a profile, and neither has a DevTools port (#1137,
//! #1200): each runs with `--remote-debugging-pipe`, its stream handed to the browser service
//! (`yantrik-ui`'s `browser_pipe`). A port is reachable by every process of the machine, and a job
//! that speaks raw DevTools to it goes round the fence the service puts on a browser's tabs.
//!
//! * the person's Browser: their own signed-in profile, which is the browser's own default (no
//!   `--user-data-dir`: Chrome 136 and later refuse DevTools on the default directory when it is
//!   named). Only their own `yos web` and `yos act browser` reach it.
//! * Mind View's browser: a profile of its own. Every mind's web tools drive this one and nothing
//!   else.
//!
//! The paths live in `browsers.conf` (beside `Cargo.toml`), which the Python browser service reads
//! at run time and `build.rs` turns into the constants below. `PERSON_PORT` is the number the
//! person's Browser used to listen on, kept so a listener there can be refused by name; Mind View's
//! old number is spelled nowhere. `listener` tells which browser holds a port, for those refusals.

use std::path::{Path, PathBuf};

pub mod listener;
pub mod procs;
pub mod stream;

include!(concat!(env!("OUT_DIR"), "/ports.rs"));

/// Whose browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Person,
    MindView,
}

fn absolute(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// `$XDG_DATA_HOME`, else `~/.local/share`; the temp directory for a process with neither.
pub fn data_home() -> PathBuf {
    absolute("XDG_DATA_HOME")
        .or_else(|| absolute("HOME").map(|h| h.join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
}

/// Mind View's browser profile.
pub fn mind_view_profile() -> PathBuf {
    data_home().join(MIND_VIEW_PROFILE)
}

/// The root of the built-in companion's other browsing identities.
pub fn identities_root() -> PathBuf {
    data_home().join(IDENTITY_PROFILES)
}

/// An identity names a directory, so it may not wander out of one.
pub fn sanitize_identity(identity: &str) -> String {
    let cleaned: String = identity
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    if cleaned.is_empty() {
        "default".to_string()
    } else {
        cleaned
    }
}

/// The profile of the companion's browsing identity `identity`: the default one is Mind View's
/// browser, the others each have a directory of their own. They run with no DevTools switch.
pub fn identity_profile(identity: &str) -> PathBuf {
    let identity = sanitize_identity(identity);
    if identity == "default" {
        return mind_view_profile();
    }
    identities_root().join(identity)
}

/// The switch that gives a Chromium its DevTools over fds 3 (it reads) and 4 (it writes) and
/// opens no port (#1137, #1200). Both of the desktop's browsers run with this.
pub const PIPE_FLAG: &str = "--remote-debugging-pipe";

/// What makes a Chromium-family browser the browser of `side`: the pipe, and no port at all (so
/// nothing of either listens on TCP). The person's has no profile: it runs on its default one, so
/// nothing of theirs changes and Chrome 136+ keeps DevTools on. Mind View's has its own
/// `--user-data-dir`, and `AutomationControlled` off: a pipe, unlike a port, would otherwise
/// make `navigator.webdriver` true, which a bot check reads and a port never showed it.
pub fn launch_flags(side: Side) -> Vec<String> {
    match side {
        Side::Person => vec![PIPE_FLAG.into()],
        Side::MindView => vec![
            PIPE_FLAG.into(),
            "--disable-blink-features=AutomationControlled".into(),
            // After a crash Chromium offers to restore the last session; a mind's browser is never
            // offered it (the shell also removes the session files before the start).
            "--disable-session-crashed-bubble".into(),
            format!("--user-data-dir={}", mind_view_profile().display()),
        ],
    }
}

/// Whose browser a process is, by what it owns and never by what its arguments say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// It owns Mind View's profile.
    MindView,
    /// It owns one of the companion's identity profiles.
    Identity,
    /// It owns neither: the person's, or nobody's we know.
    Person,
}

impl Place {
    /// Whether this is a browser of a mind.
    pub fn is_mind_side(self) -> bool {
        matches!(self, Place::MindView | Place::Identity)
    }
}

/// The profile directories that make a browser a mind's.
#[derive(Clone, Debug)]
pub struct Profiles {
    pub mind_view: PathBuf,
    pub identities: PathBuf,
}

impl Profiles {
    pub fn current() -> Profiles {
        Profiles { mind_view: mind_view_profile(), identities: identities_root() }
    }

    /// Every directory that is a mind's profile now.
    fn dirs(&self) -> Vec<(PathBuf, Place)> {
        let mut out = vec![(self.mind_view.clone(), Place::MindView)];
        if let Ok(entries) = std::fs::read_dir(&self.identities) {
            out.extend(entries.flatten().filter(|e| e.path().is_dir()).map(|e| (e.path(), Place::Identity)));
        }
        out
    }
}

/// The pid a profile's `SingletonLock` names. Chromium writes it (a link to `host-pid`) when it
/// takes the profile; nothing in a command line does.
pub fn lock_pid(profile: &Path) -> Option<u32> {
    let link = std::fs::read_link(profile.join("SingletonLock")).ok()?;
    link.to_str()?.rsplit('-').next()?.parse().ok()
}

/// Whether process `pid` owns `profile`: its `SingletonLock` names it, or it holds one of the
/// profile's `LOCK` files open (an exclusive lock on a database in it). Errs when the files the
/// process holds cannot be read.
pub fn owns(proc: &Path, pid: u32, profile: &Path) -> Result<bool, String> {
    if lock_pid(profile) == Some(pid) {
        return Ok(true);
    }
    let real = std::fs::canonicalize(profile).unwrap_or_else(|_| profile.to_path_buf());
    let fds = std::fs::read_dir(proc.join(pid.to_string()).join("fd"))
        .map_err(|e| format!("the files process {pid} holds could not be read ({e})"))?;
    Ok(fds.flatten().any(|fd| {
        let Ok(held) = std::fs::read_link(fd.path()) else { return false };
        let held = held.to_string_lossy().trim_end_matches(" (deleted)").to_string();
        let held = PathBuf::from(held);
        held.file_name().is_some_and(|n| n == "LOCK")
            && std::fs::canonicalize(&held).unwrap_or(held).starts_with(&real)
    }))
}

/// Whose browser process `pid` is, in `proc` (a `/proc`).
pub fn place_in(proc: &Path, profiles: &Profiles, pid: u32) -> Result<Place, String> {
    for (dir, place) in profiles.dirs() {
        if owns(proc, pid, &dir)? {
            return Ok(place);
        }
    }
    Ok(Place::Person)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_only_port_left_is_the_persons_old_one() {
        assert_eq!(PERSON_PORT, 9222);
    }

    #[test]
    fn the_default_identity_is_mind_views_browser_and_the_others_have_a_directory_each() {
        assert_eq!(identity_profile("default"), mind_view_profile());
        assert_eq!(identity_profile("  "), identity_profile("default"));
        for name in ["work", "personal", "a/b", "../x", "x".repeat(40).as_str()] {
            let profile = identity_profile(name);
            assert!(profile.starts_with(identities_root()), "{name}: {profile:?}");
            assert_eq!(identity_profile(name), profile);
        }
    }

    #[test]
    fn neither_launch_names_a_port_and_mind_views_has_its_own_profile() {
        for side in [Side::Person, Side::MindView] {
            let flags = launch_flags(side);
            assert!(flags.contains(&PIPE_FLAG.to_string()), "{side:?}: the pipe");
            assert!(!flags.iter().any(|f| f.contains("port") || f.contains("origins") || f.contains("address")), "{side:?}: {flags:?}");
        }
        assert_eq!(launch_flags(Side::Person), vec![PIPE_FLAG.to_string()], "the pipe and nothing else of DevTools");
        let mind = launch_flags(Side::MindView);
        assert!(mind.iter().any(|f| f.ends_with(MIND_VIEW_PROFILE)), "{mind:?}");
        assert!(mind.contains(&"--disable-blink-features=AutomationControlled".to_string()));
    }

    #[test]
    fn a_lock_names_its_pid_and_nothing_else_does() {
        let dir = std::env::temp_dir().join(format!("yb-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(lock_pid(&dir), None);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("my-host-4242", dir.join("SingletonLock")).unwrap();
            assert_eq!(lock_pid(&dir), Some(4242));
            assert_eq!(owns(Path::new("/nonexistent"), 4242, &dir), Ok(true));
            assert!(owns(Path::new("/nonexistent"), 4243, &dir).is_err(), "the files of a process nobody can see");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
