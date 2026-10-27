//! Who asked a running copy for a window: the person, or a Mind (#978 step 1).
//!
//! A hand-over (`launch`, `present`) names the display it came from, but a name in a request is
//! only a claim. The asking process is still connected while it waits for its answer, so the
//! kernel's word on its pid (`SO_PEERCRED`, [`crate::control::caller`]) leads to its own
//! environment, `/proc/<pid>/environ`: the `WAYLAND_DISPLAY` it was started with and the marker
//! the shell sets on every launch onto a mind's display. That is what decides.
//!
//! The claim can only make the asker a Mind's, never the person's, so a launch that lost its
//! marker on the way is still a Mind's, and nothing a request says turns a Mind's launch into a
//! window of the person's. A caller of another account is never the person. On a build that is not
//! Linux, or with no socket call at all (a test), the claim is all there is, and it
//! is held to the same rule as the environment: [`crate::for_mind::drawn_for_mind`]. A caller
//! the kernel names whose environment cannot be read on Linux is a Mind's: it proves nothing.

use serde_json::Value;

use crate::for_mind::{self, FOR_MIND_VAR};

/// What is known of the process that asked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Asker {
    /// Its `WAYLAND_DISPLAY`, when it had one.
    pub display: Option<String>,
    /// Whether it carries the marker [`FOR_MIND_VAR`] (any value but an empty one).
    pub marker: bool,
    /// Whether this was read from its environment rather than taken from its request.
    pub verified: bool,
}

impl Asker {
    /// Whether it asks as a Mind: it carries the marker, or its display is not the person's
    /// (#959's fail-closed rule).
    pub fn is_mind(&self) -> bool {
        let marker = self.marker.then_some("1");
        for_mind::drawn_for_mind(marker, self.display.as_deref())
    }

    /// What a request says of itself: `display` and `for_mind`, as `launch::hand_over` and
    /// `present::ask` send them.
    pub fn claimed(args: &Value) -> Asker {
        Asker {
            display: args["display"].as_str().filter(|d| !d.is_empty()).map(str::to_string),
            marker: args["for_mind"].as_bool().unwrap_or(false),
            verified: false,
        }
    }

    /// From an environment block as `/proc/<pid>/environ` holds it: `KEY=value` entries, each
    /// ended by a NUL.
    pub fn from_environ(block: &[u8]) -> Asker {
        let mut asker = Asker { verified: true, ..Asker::default() };
        for entry in block.split(|b| *b == 0) {
            let entry = String::from_utf8_lossy(entry);
            let Some((key, value)) = entry.split_once('=') else { continue };
            match key {
                "WAYLAND_DISPLAY" if !value.is_empty() => asker.display = Some(value.to_string()),
                k if k == FOR_MIND_VAR && !value.is_empty() => asker.marker = true,
                _ => {}
            }
        }
        asker
    }

    /// This asker, made a Mind's when `claim` is one: the verified environment decides, and a
    /// claim may only add the Mind to it.
    pub fn or_mind(self, claim: &Asker) -> Asker {
        if self.is_mind() || !claim.is_mind() {
            return self;
        }
        Asker { display: claim.display.clone().or(self.display), marker: true, verified: self.verified }
    }
}

/// Who asked for the call being handled on this thread, given its request `args`.
pub fn asking(args: &Value) -> Asker {
    let claim = Asker::claimed(args);
    let Some(caller) = crate::control::caller() else {
        // No socket call at all: the framework's own hop, or a test. The request is all there is.
        return claim;
    };
    if caller.uid != own_uid() {
        // Another account's process is never the person's own launch.
        return Asker { marker: true, ..claim };
    }
    match read_environ(caller.pid) {
        Some(block) => Asker::from_environ(&block).or_mind(&claim),
        // A caller the kernel names whose environment cannot be read (it has gone, its pid was
        // reused, it is not dumpable) proves nothing: fail closed (#972 L1).
        None if cfg!(target_os = "linux") => Asker { marker: true, ..claim },
        None => claim,
    }
}

#[cfg(unix)]
fn own_uid() -> u32 {
    // SAFETY: geteuid cannot fail and touches no memory.
    unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn own_uid() -> u32 {
    0
}

/// The environment `pid` was started with, while it is alive and ours to read.
fn read_environ(pid: i32) -> Option<Vec<u8>> {
    if pid <= 0 || !cfg!(target_os = "linux") {
        return None;
    }
    std::fs::read(format!("/proc/{pid}/environ")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::for_mind::PERSON_DISPLAY;
    use serde_json::json;

    fn block(entries: &[&str]) -> Vec<u8> {
        entries.iter().flat_map(|e| e.bytes().chain([0])).collect()
    }

    #[test]
    fn the_environment_names_the_display_and_the_marker() {
        let person = Asker::from_environ(&block(&["HOME=/home/p", "WAYLAND_DISPLAY=wayland-0"]));
        assert_eq!(person.display.as_deref(), Some(PERSON_DISPLAY));
        assert!(!person.marker && person.verified);
        assert!(!person.is_mind());

        let mind = Asker::from_environ(&block(&["WAYLAND_DISPLAY=wayland-1", "YANTRIK_FOR_MIND=1"]));
        assert!(mind.is_mind());
        // The display alone decides when the marker was lost on the way (#959's fail-closed rule).
        let lost = Asker::from_environ(&block(&["WAYLAND_DISPLAY=wayland-1"]));
        assert!(lost.is_mind());
        // An empty marker is no marker; any other value is one.
        assert!(!Asker::from_environ(&block(&["YANTRIK_FOR_MIND=", "WAYLAND_DISPLAY=wayland-0"])).is_mind());
        assert!(Asker::from_environ(&block(&["YANTRIK_FOR_MIND=0", "WAYLAND_DISPLAY=wayland-0"])).is_mind());
    }

    /// A Mind's launch can never claim the person's display: what its environment says stands.
    #[test]
    fn a_claim_can_make_the_asker_a_minds_and_never_the_persons() {
        let mind = Asker::from_environ(&block(&["WAYLAND_DISPLAY=wayland-1", "YANTRIK_FOR_MIND=1"]));
        let claims_person = Asker::claimed(&json!({ "display": PERSON_DISPLAY }));
        assert!(mind.clone().or_mind(&claims_person).is_mind());

        let person = Asker::from_environ(&block(&["WAYLAND_DISPLAY=wayland-0"]));
        let claims_mind = Asker::claimed(&json!({ "display": "wayland-1", "for_mind": true }));
        assert!(person.clone().or_mind(&claims_mind).is_mind());
        assert!(!person.or_mind(&claims_person).is_mind());
    }

    #[test]
    fn a_claim_alone_is_held_to_the_same_rule() {
        assert!(!Asker::claimed(&json!({ "display": PERSON_DISPLAY })).is_mind());
        assert!(!Asker::claimed(&json!({})).is_mind(), "no display named: not on Wayland");
        assert!(Asker::claimed(&json!({ "display": "wayland-1" })).is_mind());
        assert!(Asker::claimed(&json!({ "display": PERSON_DISPLAY, "for_mind": true })).is_mind());
        assert!(!Asker::claimed(&json!({ "display": "wayland-1" })).verified);
    }

    /// Outside a socket call there is no caller, and the request is what is known.
    #[test]
    fn with_no_caller_the_request_is_taken() {
        let asked = asking(&json!({ "display": "wayland-1", "for_mind": true }));
        assert!(asked.is_mind() && !asked.verified);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_reads_its_own_environment() {
        let me = read_environ(std::process::id() as i32).expect("our own environ");
        assert!(!me.is_empty());
        assert!(read_environ(0).is_none() && read_environ(-1).is_none());
    }
}
