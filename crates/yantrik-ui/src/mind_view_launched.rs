//! The apps the shell launched in Mind View for a Mind are that Mind's (#1027).
//!
//! They are children of the shell, so a caller found among them has no attached harness above it
//! and used to read as the person. A caller counts as the person only when nothing says
//! otherwise, and a recorded Mind View launch says otherwise: a caller that is, or descends from,
//! a process recorded here is that Mind's.
//!
//! A record names the process, not only its number: pid plus start time (`/proc/<pid>/stat`,
//! field 22), so a pid the system reuses after the app exits does not inherit the Mind. The
//! record is removed when the app exits.

use std::sync::Mutex;

use yantrik_ipc_transport::peer_identity::{self, ProcessFacts};

/// What is said of a Mind whose name the launch did not carry.
const UNNAMED: &str = "a mind";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub pid: u32,
    /// Start time in clock ticks; `None` when `/proc` could not be read at launch.
    pub started: Option<u64>,
    pub mind: String,
}

static LAUNCHES: Mutex<Vec<Launch>> = Mutex::new(Vec::new());

fn with_launches<T>(f: impl FnOnce(&mut Vec<Launch>) -> T) -> T {
    f(&mut LAUNCHES.lock().unwrap_or_else(|p| p.into_inner()))
}

/// Record a process just launched in Mind View.
pub fn record(pid: u32) {
    let started = peer_identity::walk(pid as i32).first().filter(|f| f.pid == pid as i32).map(|f| f.started);
    with_launches(|l| {
        l.retain(|x| x.pid != pid);
        l.push(Launch { pid, started, mind: UNNAMED.to_string() });
    });
}

/// Forget a launch whose app has exited.
pub fn forget(pid: u32) {
    with_launches(|l| l.retain(|x| x.pid != pid));
}

/// The Mind a caller with this pid belongs to by the apps launched for it, if any.
pub fn owner_of(pid: i32) -> Option<String> {
    let launches = with_launches(|l| l.clone());
    if launches.is_empty() {
        return None; // no /proc walk when nothing was launched
    }
    owner_in(&peer_identity::walk(pid), &launches)
}

/// Pure: the Mind of the first launch the chain (caller first, then its parents) contains.
pub fn owner_in(chain: &[ProcessFacts], launches: &[Launch]) -> Option<String> {
    chain.iter().find_map(|p| {
        launches
            .iter()
            .find(|l| l.pid as i32 == p.pid && l.started.is_none_or(|s| s == p.started))
            .map(|l| l.mind.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind_view::{classify, CallerFacts, Requester};

    fn proc_at(pid: i32, started: u64) -> ProcessFacts {
        ProcessFacts { pid, exe: String::new(), short_cmdline: String::new(), started }
    }

    fn launch(pid: u32, started: u64) -> Launch {
        Launch { pid, started: Some(started), mind: "pi".to_string() }
    }

    /// What `requester_now` does with the answer: a caller no harness claims is the person unless
    /// a launch says otherwise.
    fn classified(chain: &[ProcessFacts], launches: &[Launch]) -> Requester {
        let facts = CallerFacts {
            pid: Some(chain[0].pid as u32),
            agent: None,
            attached_mind: owner_in(chain, launches),
            mind_account: false,
        };
        classify(&facts, 1)
    }

    // Fake table: shell 100 started app 200 for a Mind; app 200 started helper 300; the person
    // opened app 400 from a terminal.
    #[test]
    fn an_app_launched_for_a_mind_and_its_child_are_that_minds() {
        let launches = [launch(200, 5000)];
        let app = [proc_at(200, 5000), proc_at(100, 10)];
        let child = [proc_at(300, 5100), proc_at(200, 5000), proc_at(100, 10)];
        assert_eq!(classified(&app, &launches), Requester::Mind("pi".into()));
        assert_eq!(classified(&child, &launches), Requester::Mind("pi".into()));
    }

    #[test]
    fn an_app_the_person_opened_is_the_person() {
        let launches = [launch(200, 5000)];
        let theirs = [proc_at(400, 6000), proc_at(100, 10)];
        assert_eq!(classified(&theirs, &launches), Requester::Person);
        assert_eq!(classified(&theirs, &[]), Requester::Person);
    }

    #[test]
    fn a_reused_pid_does_not_inherit_the_mind() {
        let launches = [launch(200, 5000)];
        let reused = [proc_at(200, 9000), proc_at(100, 10)];
        assert_eq!(classified(&reused, &launches), Requester::Person);
    }

    #[test]
    fn a_launch_is_forgotten_when_the_app_exits() {
        record(u32::MAX - 7);
        assert!(with_launches(|l| l.iter().any(|x| x.pid == u32::MAX - 7)));
        forget(u32::MAX - 7);
        assert!(!with_launches(|l| l.iter().any(|x| x.pid == u32::MAX - 7)));
    }
}
