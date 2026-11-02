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
//! The numbers and paths live in `apps/browser/yantrik_browser/browsers.conf`, which the Python
//! browser service reads at run time and `build.rs` turns into the constants below. A mind is
//! sent to its port by who it is; the port is then checked by what holds it (`listener`), never
//! by who answers first.

use std::path::PathBuf;

pub mod cmdline;
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

/// What makes a Chromium-family browser the browser of `side`: its port and its origin, and for
/// Mind View's its own `--user-data-dir`. The person's names no profile: it runs on its default
/// one, so nothing of theirs changes and Chrome 136+ keeps DevTools on.
pub fn launch_flags(side: Side) -> Vec<String> {
    let mut flags = debugging_flags(side.port());
    if side == Side::MindView {
        flags.push(format!("--user-data-dir={}", mind_view_profile().display()));
    }
    flags
}

/// Whose profile a browser was started with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// No `--user-data-dir`: the browser's default profile, which is the person's.
    PersonDefault,
    /// Mind View's profile.
    MindView,
    /// One of the companion's other identities.
    Identity,
    /// A profile named some other way: the person's (their default spelled out) or a throwaway.
    Other,
}

impl Place {
    /// Whether this is a browser of a mind, by its profile.
    pub fn is_mind_side(self) -> bool {
        matches!(self, Place::MindView | Place::Identity)
    }
}

/// Whose profile a command line (a `/proc/<pid>/cmdline`, either way written) names. None when
/// it names its profile twice or with nothing: not guessed at.
pub fn place_of(raw: &[u8]) -> Option<Place> {
    use cmdline::Profile;
    match cmdline::profile(raw) {
        Profile::Unclear => None,
        Profile::Default => Some(Place::PersonDefault),
        Profile::Named(first_word) => {
            let root = identities_root();
            Some(if cmdline::has_profile(raw, &mind_view_profile()) {
                Place::MindView
            } else if first_word.starts_with(&format!("{}/", root.display())) {
                Place::Identity
            } else {
                Place::Other
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
        assert!(person.contains(&format!("--remote-debugging-port={PERSON_PORT}")));
        assert!(!person.iter().any(|f| f.starts_with("--user-data-dir")), "{person:?}");
        let mind = launch_flags(Side::MindView);
        assert!(mind.contains(&format!("--remote-debugging-port={MIND_VIEW_PORT}")));
        assert!(mind.contains(&format!("--remote-allow-origins=http://127.0.0.1:{MIND_VIEW_PORT}")));
        assert!(mind.iter().any(|f| f.ends_with(MIND_VIEW_PROFILE)), "{mind:?}");
        assert!(!mind.iter().any(|f| f.contains(&PERSON_PORT.to_string())));
    }

    #[test]
    fn a_profile_is_told_by_its_whole_path() {
        let line = |dir: &std::path::Path| format!("chromium --user-data-dir={} about:blank", dir.display()).into_bytes();
        assert_eq!(place_of(b"chromium --remote-debugging-port=9222"), Some(Place::PersonDefault));
        assert_eq!(place_of(&line(&mind_view_profile())), Some(Place::MindView));
        assert_eq!(place_of(&line(&identities_root().join("work"))), Some(Place::Identity));
        assert_eq!(place_of(&line(Path::new("/home/p/.config/chromium"))), Some(Place::Other));
        assert_eq!(place_of(b"chromium --user-data-dir=/a --user-data-dir=/b"), None);
    }
}
