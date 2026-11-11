//! Which DevTools port and profile each of the desktop's browsers uses, and how to tell which
//! browser holds a port (#1085).
//!
//! There are two browsers and they never share a port or a profile:
//!
//! * the person's Browser: `PERSON_PORT`, their own signed-in profile, which is the browser's own
//!   default (no `--user-data-dir`: Chrome 136 and later refuse DevTools on the default directory
//!   when it is named). Only their own `yos web` and `yos act browser` reach it.
//! * Mind View's browser: `MIND_VIEW_PORT`, a profile of its own. Every mind's web tools, the
//!   built-in companion's included, drive this one and nothing else.
//!
//! The numbers and paths live in `browsers.conf` (beside `Cargo.toml`), which the Python
//! browser service reads at run time and `build.rs` turns into the constants below. A mind is
//! sent to its port by who it is; the port is then checked by what holds it (`listener`), never
//! by who answers first.

use std::path::{Path, PathBuf};

pub mod listener;
pub mod procs;

include!(concat!(env!("OUT_DIR"), "/ports.rs"));

/// Whose browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Person,
    MindView,
}

impl Side {
    pub fn port(self) -> u16 {
        match self {
            Side::Person => PERSON_PORT,
            Side::MindView => MIND_VIEW_PORT,
        }
    }
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

/// The companion's browsing identity `identity`: the default one is Mind View's browser, the
/// others each have a profile of their own and a port in the identity range.
pub fn identity_side(identity: &str) -> (u16, PathBuf) {
    let identity = sanitize_identity(identity);
    if identity == "default" {
        return (MIND_VIEW_PORT, mind_view_profile());
    }
    // FNV-1a, so the same name always lands on the same port and a caller can find it again.
    let mut hash: u32 = 2166136261;
    for byte in identity.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    let span = (IDENTITY_PORT_LAST - IDENTITY_PORT_FIRST + 1) as u32;
    (IDENTITY_PORT_FIRST + (hash % span) as u16, identities_root().join(identity))
}

/// The three switches that make a Chromium a browser the desktop's tools can drive: loopback
/// only, and only the origin the desktop's own client presents.
pub fn debugging_flags(port: u16) -> Vec<String> {
    vec![
        "--remote-debugging-address=127.0.0.1".into(),
        format!("--remote-debugging-port={port}"),
        format!("--remote-allow-origins=http://127.0.0.1:{port}"),
    ]
}

/// The switch that gives a Chromium its DevTools over fds 3 (it reads) and 4 (it writes) and
/// opens no port (#1137). The person's Browser runs with this and with nothing else of DevTools.
pub const PIPE_FLAG: &str = "--remote-debugging-pipe";

/// What makes a Chromium-family browser the browser of `side`. Mind View's: its port, its origin
/// and its own `--user-data-dir`. The person's: the pipe, no port at all (so nothing of theirs
/// listens on TCP, and `PERSON_PORT` is only the number an older build used), and no profile: it
/// runs on its default one, so nothing of theirs changes and Chrome 136+ keeps DevTools on.
pub fn launch_flags(side: Side) -> Vec<String> {
    if side == Side::Person {
        return vec![PIPE_FLAG.into()];
    }
    let mut flags = debugging_flags(side.port());
    flags.push(format!("--user-data-dir={}", mind_view_profile().display()));
    flags
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
    fn the_ports_are_the_decided_ones() {
        assert_eq!((PERSON_PORT, MIND_VIEW_PORT), (9222, 9223));
        assert_eq!(Side::Person.port(), PERSON_PORT);
        assert_eq!(Side::MindView.port(), MIND_VIEW_PORT);
    }

    #[test]
    fn the_default_identity_is_mind_views_browser_and_the_others_stay_in_range() {
        let (port, profile) = identity_side("default");
        assert_eq!((port, profile), (MIND_VIEW_PORT, mind_view_profile()));
        assert_eq!(identity_side("  "), identity_side("default"));
        for name in ["work", "personal", "a/b", "../x", "x".repeat(40).as_str()] {
            let (port, profile) = identity_side(name);
            assert!((IDENTITY_PORT_FIRST..=IDENTITY_PORT_LAST).contains(&port), "{name}: {port}");
            assert!(port != PERSON_PORT && port != MIND_VIEW_PORT);
            assert!(profile.starts_with(identities_root()), "{name}: {profile:?}");
            assert_eq!(identity_side(name), (port, profile));
        }
    }

    #[test]
    fn the_persons_launch_names_no_profile_and_mind_views_its_own() {
        let person = launch_flags(Side::Person);
        assert_eq!(person, vec![PIPE_FLAG.to_string()], "the pipe and nothing else of DevTools");
        assert!(!person.iter().any(|f| f.contains("port") || f.contains("9222")), "{person:?}");
        let mind = launch_flags(Side::MindView);
        assert!(mind.contains(&format!("--remote-debugging-port={MIND_VIEW_PORT}")));
        assert!(mind.contains(&format!("--remote-allow-origins=http://127.0.0.1:{MIND_VIEW_PORT}")));
        assert!(mind.iter().any(|f| f.ends_with(MIND_VIEW_PROFILE)), "{mind:?}");
        assert!(!mind.iter().any(|f| f.contains(&PERSON_PORT.to_string())));
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
