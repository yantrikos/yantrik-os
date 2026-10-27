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
//!
//! **What this does not do.** Step 1 of design #978 decides person against Mind from the asking
//! process's environment and display, and from which program it is. A Mind's `agent_run` that
//! rewrites its own environment (`env -u YANTRIK_FOR_MIND WAYLAND_DISPLAY=<the person's>`) and
//! runs a Yantrik app binary can still pose as the person. Identity that a caller cannot claim
//! comes from sessions the shell creates (design section 1, step 2), which closes this. The program check
//! below ([`exe_is_ours`]) is a cheap guard, not that fix. The display rule has no X11 side either:
//! a process with `DISPLAY=:N` (Mind View's Xwayland) and no `WAYLAND_DISPLAY` is 'not on Wayland'
//! and counts as the person's. Step 2 closes that with the rest.

use serde_json::Value;

use yantrik_ipc_contracts::mind_view_displays::Known;

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
        self.is_mind_in(&for_mind::known_displays())
    }

    /// [`Asker::is_mind`] against the displays `known`.
    pub fn is_mind_in(&self, known: &Known) -> bool {
        let marker = self.marker.then_some("1");
        for_mind::drawn_for_mind_in(marker, self.display.as_deref(), known)
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
        self.or_mind_in(claim, &for_mind::known_displays())
    }

    /// [`Asker::or_mind`] against the displays `known`.
    pub fn or_mind_in(self, claim: &Asker, known: &Known) -> Asker {
        if self.is_mind_in(known) || !claim.is_mind_in(known) {
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
    // Only a Yantrik app binary or the shell hands a launch over; anything else is a Mind's.
    if !exe_is_ours(caller.pid) {
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

/// Whether `exe`, the path a process runs, is one of Yantrik's own programs: a `yantrik-*` file in
/// the directory `own` (this process's program) is in, which is where the app binaries and the shell
/// are installed. A binary replaced by an update reads as `... (deleted)`. A file named like one
/// that sits anywhere else (`cp /usr/bin/python3 ~/yantrik-x`) is not.
pub fn is_our_program(exe: &str, own: &str) -> bool {
    let strip = |p: &str| p.strip_suffix(" (deleted)").unwrap_or(p).to_string();
    let (exe, own) = (strip(exe), strip(own));
    let (exe, own) = (std::path::Path::new(&exe), std::path::Path::new(&own));
    exe.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("yantrik-"))
        && exe.parent().is_some()
        && exe.parent() == own.parent()
}

/// Whether the caller's program `theirs` may hand a launch over to this process, whose own program
/// is `own`: the very same program (the app itself, a second copy of it), or another Yantrik
/// program beside it ([`is_our_program`]). There is no switch for tests: a test that calls across
/// a socket from its own process is the same program, and one that calls from another binary
/// injects the allowance at runtime, never through `cfg`.
pub fn may_hand_over(theirs: &str, own: &str) -> bool {
    theirs == own || is_our_program(theirs, own)
}

/// Whether the process `pid` runs a program that may hand a launch over. Off Linux there is no way
/// to ask, and the claim is all there is; on Linux a process whose program cannot be read, or this
/// process's own, is not ours.
fn exe_is_ours(pid: i32) -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    let read = |p: String| std::fs::read_link(p).ok().and_then(|l| l.to_str().map(str::to_string));
    match (read(format!("/proc/{pid}/exe")), read("/proc/self/exe".to_string())) {
        (Some(theirs), Some(own)) => may_hand_over(&theirs, &own),
        _ => false,
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
    const PERSON_DISPLAY: &str = "wayland-0";

    fn k() -> Known {
        Known { person: Some(PERSON_DISPLAY.to_string()), minds: Vec::new() }
    }
    use serde_json::json;

    fn block(entries: &[&str]) -> Vec<u8> {
        entries.iter().flat_map(|e| e.bytes().chain([0])).collect()
    }

    #[test]
    fn the_environment_names_the_display_and_the_marker() {
        let person = Asker::from_environ(&block(&["HOME=/home/p", "WAYLAND_DISPLAY=wayland-0"]));
        assert_eq!(person.display.as_deref(), Some(PERSON_DISPLAY));
        assert!(!person.marker && person.verified);
        assert!(!person.is_mind_in(&k()));

        let mind = Asker::from_environ(&block(&["WAYLAND_DISPLAY=wayland-1", "YANTRIK_FOR_MIND=1"]));
        assert!(mind.is_mind_in(&k()));
        // The display alone decides when the marker was lost on the way (#959's fail-closed rule).
        let lost = Asker::from_environ(&block(&["WAYLAND_DISPLAY=wayland-1"]));
        assert!(lost.is_mind_in(&k()));
        // An empty marker is no marker; any other value is one.
        assert!(!Asker::from_environ(&block(&["YANTRIK_FOR_MIND=", "WAYLAND_DISPLAY=wayland-0"])).is_mind_in(&k()));
        assert!(Asker::from_environ(&block(&["YANTRIK_FOR_MIND=0", "WAYLAND_DISPLAY=wayland-0"])).is_mind_in(&k()));
    }

    /// A Mind's launch can never claim the person's display: what its environment says stands.
    #[test]
    fn a_claim_can_make_the_asker_a_minds_and_never_the_persons() {
        let mind = Asker::from_environ(&block(&["WAYLAND_DISPLAY=wayland-1", "YANTRIK_FOR_MIND=1"]));
        let claims_person = Asker::claimed(&json!({ "display": PERSON_DISPLAY }));
        assert!(mind.clone().or_mind_in(&claims_person, &k()).is_mind_in(&k()));

        let person = Asker::from_environ(&block(&["WAYLAND_DISPLAY=wayland-0"]));
        let claims_mind = Asker::claimed(&json!({ "display": "wayland-1", "for_mind": true }));
        assert!(person.clone().or_mind_in(&claims_mind, &k()).is_mind_in(&k()));
        assert!(!person.or_mind_in(&claims_person, &k()).is_mind_in(&k()));
    }

    #[test]
    fn a_claim_alone_is_held_to_the_same_rule() {
        assert!(!Asker::claimed(&json!({ "display": PERSON_DISPLAY })).is_mind_in(&k()));
        assert!(!Asker::claimed(&json!({})).is_mind_in(&k()), "no display named: not on Wayland");
        assert!(Asker::claimed(&json!({ "display": "wayland-1" })).is_mind_in(&k()));
        assert!(Asker::claimed(&json!({ "display": PERSON_DISPLAY, "for_mind": true })).is_mind_in(&k()));
        assert!(!Asker::claimed(&json!({ "display": "wayland-1" })).verified);
    }

    /// Outside a socket call there is no caller, and the request is what is known.
    #[test]
    fn with_no_caller_the_request_is_taken() {
        let asked = asking(&json!({ "display": "wayland-1", "for_mind": true }));
        assert!(asked.is_mind_in(&k()) && !asked.verified);
    }

    /// A person on any display, not only `wayland-0`, is the person; a Mind on its seat is not.
    #[test]
    fn a_person_on_another_display_is_the_person_and_a_mind_on_its_seat_is_not() {
        let known = Known { person: Some("wayland-1".into()), minds: vec!["wayland-2".into()] };
        let person = Asker::from_environ(&block(&["WAYLAND_DISPLAY=wayland-1"]));
        assert!(!person.is_mind_in(&known));
        let mind = Asker::from_environ(&block(&["WAYLAND_DISPLAY=wayland-2"]));
        assert!(mind.is_mind_in(&known));
        let claims_person = Asker::claimed(&json!({ "display": "wayland-1" }));
        assert!(mind.or_mind_in(&claims_person, &known).is_mind_in(&known), "a claim never makes a Mind the person");
    }

    /// Only Yantrik's own programs hand a launch over (M1): anything else is a Mind's.
    #[test]
    fn only_a_yantrik_program_may_hand_a_launch_over() {
        let own = "/opt/yantrik/bin/yantrik-notes";
        // The same program, a second copy of the app itself; and Yantrik's own beside it.
        for ours in [own, "/opt/yantrik/bin/yantrik-ui", "/opt/yantrik/bin/yantrik-notes (deleted)"] {
            assert!(may_hand_over(ours, own), "{ours}");
        }
        assert!(may_hand_over("/x/target/debug/yantrik-email (deleted)", "/x/target/debug/yantrik-notes"));
        for other in [
            "/usr/bin/python3",
            "/usr/bin/env",
            "/home/p/yantrik-notes/evil",
            "/tmp/notes",
            "/usr/bin/foot",
            "",
            // A copy of anything, named like one of ours, anywhere else.
            "/home/p/yantrik-x",
            "/tmp/yantrik-notes",
        ] {
            assert!(!may_hand_over(other, own), "{other}");
        }
    }

    /// The same program is accepted whatever it is called: a test calling across a socket from its
    /// own process needs nothing else.
    #[test]
    fn the_same_program_is_the_app_itself_whatever_it_is_called() {
        assert!(may_hand_over("/tmp/target/debug/deps/service-3f2a", "/tmp/target/debug/deps/service-3f2a"));
        assert!(!may_hand_over("/tmp/target/debug/deps/other-3f2a", "/tmp/target/debug/deps/service-3f2a"));
        #[cfg(target_os = "linux")]
        assert!(exe_is_ours(std::process::id() as i32), "this process is the same program as itself");
    }

    /// Shipped code has no switch: nothing in this module's code, before its tests, reads a feature or
    /// `cfg(test)`, so a binary built beside the tests (`--all-targets`) checks the program as one
    /// built alone does.
    #[test]
    fn the_program_check_has_no_feature_switch() {
        let source = include_str!("asker.rs");
        let shipped = source.split("#[cfg(test)]").next().unwrap();
        let code: String = shipped.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for switch in ["feature =", "cfg!(test)", "cfg(test)", "cfg(feature", "testing"] {
            assert!(!code.contains(switch), "shipped code reads `{switch}`");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_reads_its_own_environment() {
        let me = read_environ(std::process::id() as i32).expect("our own environ");
        assert!(!me.is_empty());
        assert!(read_environ(0).is_none() && read_environ(-1).is_none());
    }
}
