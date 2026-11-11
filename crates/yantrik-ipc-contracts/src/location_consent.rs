//! Whether this machine may ask the network where it is: off until the person says so.
//!
//! A place found from an IP address is a request that reveals the machine's public IP to a
//! third party (the desktop's own lookup asks ipinfo.io; a weather query with no place named
//! asks wttr.in, which finds the caller server-side). Nothing here makes either request. This is
//! the one answer every caller checks first, so the desktop, the weather tool and the weather
//! watch cannot disagree about it.
//!
//! The switch is a file, `~/.config/yantrik/location-lookup`, holding `on`. It is absent on a
//! fresh machine, so a first boot with nobody at the keyboard asks nobody anything. A feature
//! that needs a place and finds the switch off asks the person first, in [`ASK`]'s words.
//! `YANTRIK_NO_GEOLOCATE=1` outranks everything: when set, the answer is no even when asked.

use std::path::{Path, PathBuf};

/// What a feature says before it asks: what is sent, and to whom.
pub const ASK: &str = "Finding your location from your internet address sends that address to a \
    location service (ipinfo.io for the desktop, wttr.in for weather). Say yes to allow it, or \
    name a city instead and nothing is sent.";

const ENV_OFF: &str = "YANTRIK_NO_GEOLOCATE";
const FILE: &str = "location-lookup";

/// True when `YANTRIK_NO_GEOLOCATE` says never.
pub fn env_forbids() -> bool {
    forbids(std::env::var(ENV_OFF).ok().as_deref())
}

fn forbids(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("1" | "true" | "yes" | "on"))
}

/// Where the switch lives.
pub fn switch_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".config/yantrik").join(FILE)
}

/// May an IP-based lookup be made now: the person turned it on, and nothing forbids it.
pub fn allowed() -> bool {
    !env_forbids() && switched_on(&switch_path())
}

fn switched_on(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|s| matches!(s.trim(), "on" | "1" | "yes" | "true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_machine_has_no_switch_so_nothing_is_allowed() {
        let dir = tempfile_dir("fresh");
        assert!(!switched_on(&dir.join(FILE)));
    }

    #[test]
    fn only_an_explicit_on_turns_it_on() {
        let dir = tempfile_dir("on");
        let file = dir.join(FILE);
        for (text, want) in [("on\n", true), ("1", true), ("off", false), ("", false), ("maybe", false)] {
            std::fs::write(&file, text).unwrap();
            assert_eq!(switched_on(&file), want, "{text:?}");
        }
    }

    #[test]
    fn the_environment_forbids_even_when_asked() {
        for v in ["1", "true", "yes", "on", " 1 "] {
            assert!(forbids(Some(v)), "{v:?}");
        }
        for v in [None, Some(""), Some("0"), Some("no")] {
            assert!(!forbids(v), "{v:?}");
        }
    }

    #[test]
    fn the_ask_says_what_is_sent_and_to_whom() {
        assert!(ASK.contains("internet address") && ASK.contains("ipinfo.io") && ASK.contains("wttr.in"));
    }

    fn tempfile_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-loc-consent-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
