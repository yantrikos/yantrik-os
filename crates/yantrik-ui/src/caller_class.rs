//! `caller_class`: whether a process is the person's or a mind's, as the shell reads it (#1085).
//!
//! The browser service has to send a mind's web tools to Mind View's browser and the person's own
//! to theirs. A mind such as Hermes runs as the person's account and carries no token, so the
//! account and the token cannot tell it from the person; what can is the shell's own record of
//! which minds are attached, and the walk up `/proc` from the caller's pid to one of them. That is
//! `requester_now` for a call made on the shell's socket. This answers it for a pid the browser
//! service saw on its own socket, with the same rule (`mind_view::classify`), so the choice of
//! browser and the choice of where a mind's app opens cannot disagree.
//!
//! Anything the shell cannot place is a mind: the answer `person` is only ever given to a process
//! that is not shown to descend from an attached mind and is positively the person's by
//! `mind_view_launched` (the shell's own session, or uid 0).

use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::mind_view::{classify, CallerFacts, Requester};

/// `person` or `mind` for the process `pid`. `uid` is the caller's uid as the kernel gave it on the
/// socket (`SO_PEERCRED`), or `None` when the asker has no such socket: it is never read from
/// `/proc`, where a setuid exec could make it 0 after the caller connected.
pub fn class_of(pid: i64, uid: Option<u32>) -> &'static str {
    let Ok(pid) = i32::try_from(pid) else { return "mind" };
    if pid <= 0 {
        return "mind";
    }
    let identity = crate::caller_identity::resolve(pid);
    // A caller that has exited has no chain, and a pid reused since starts a chain of someone
    // else's: neither is shown to be the person (#1085).
    if !chain_starts_at(&identity.chain, pid) {
        return "mind";
    }
    // The same rule as `requester_now`, on the walk already done: an app Mind View launched has no
    // attached ancestor, and only its cgroup says whose it is.
    let attached_mind = crate::mind_view_launched::attached_mind_given(pid as u32, identity.attached_mind, uid);
    let facts = CallerFacts { pid: Some(pid as u32), agent: None, attached_mind, mind_account: false };
    match classify(&facts, std::process::id()) {
        Requester::Person => "person",
        Requester::Mind(_) => "mind",
    }
}

/// Whether a walked ancestry starts at the process that was asked about.
fn chain_starts_at(chain: &[yantrik_ipc_transport::peer_identity::ProcessFacts], pid: i32) -> bool {
    chain.first().is_some_and(|first| first.pid == pid)
}

pub fn actions(surface: ControlSurface) -> ControlSurface {
    // The browser service starts Mind View's browser for a mind whose web call finds none: the
    // service is not the mind, so `open_app` would be routed as the person's.
    let surface = surface.action(
        Action::new(
            "open_mind_browser",
            "Start Mind View's own browser (never the person's Browser) for a mind's web tools, and              say why when it cannot. For the desktop's own services",
        )
        .risk("standard"),
        |_args| {
            let work = || crate::wire::browser_launch::open_in_mind_view().map(|()| serde_json::json!({ "started": true }));
            yantrik_app_runtime::control::answer_later(work)
                .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
                .or_else(|work| work())
        },
    );
    surface.action(
        Action::new(
            "caller_class",
            "Whether a process is the person's own or a mind's, as the desktop reads it: `class` is \
             `person` or `mind`. For the desktop's own services, which choose a browser by it",
        )
        .risk("safe")
        .arg(Param::number("pid").describe("The process id as the kernel reported it on a socket")),
        |args| {
            let pid = args["pid"].as_i64().unwrap_or(0);
            // No uid: the browser service has already refused any uid but its own (non-root), and
            // uid 0 must never short-circuit the cgroup here.
            // Off the UI thread: the browser service asks while it handles a call, and a /proc
            // walk is no business of the thread that draws the desktop.
            let work = move || Ok(serde_json::json!({ "class": class_of(pid, None) }));
            yantrik_app_runtime::control::answer_later(work)
                .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
                .or_else(|work| work())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_cannot_be_placed_is_a_mind_never_the_person() {
        assert_eq!(class_of(0, None), "mind");
        assert_eq!(class_of(-4, None), "mind");
        assert_eq!(class_of(i64::MAX, None), "mind");
    }

    /// The browser service keeps this answer for the shell without asking (`guard.is_the_shell`,
    /// which may not ask, #1080): a mind reaches the browser through the shell, so were this to
    /// change, a mind would read and drive the person's browser by way of a glance or a button.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_shell_itself_is_a_mind() {
        assert_eq!(class_of(i64::from(std::process::id()), None), "mind");
    }

    /// The hole: a pid that had exited walked to an empty chain, which classified as the person,
    /// so a queued web_go from a killed Hermes could land in the person's tab.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_caller_that_has_exited_is_a_mind_and_one_that_is_here_is_asked_about() {
        let mut gone = std::process::Command::new("true").spawn().unwrap();
        let pid = gone.id() as i64;
        gone.wait().unwrap();
        assert_eq!(class_of(pid, None), "mind", "an exited caller is not shown to be the person");
        let empty: Vec<yantrik_ipc_transport::peer_identity::ProcessFacts> = Vec::new();
        assert!(!chain_starts_at(&empty, 7));
        let here = yantrik_ipc_transport::peer_identity::walk(std::process::id() as i32);
        assert!(chain_starts_at(&here, std::process::id() as i32));
        assert!(!chain_starts_at(&here, 1), "a chain that starts at another process is not this pid's");
    }
}
